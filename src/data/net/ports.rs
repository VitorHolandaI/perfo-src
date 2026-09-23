//! Traffic charged to the ports actually carrying it, not only the ports
//! being listened on.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::data::disk::{rate, HISTORY_SAMPLES};

use super::inet_diag::SocketSample;
use super::{push_capped, PortSide, PortTraffic};

/// How many ports the table keeps.
const MAX_TRAFFIC_PORTS: usize = 32;
/// How many ports are remembered at once. Totals outlive the sockets that
/// earned them, so without a bound a long session would hold every port it
/// ever touched, each with its own rate rings.
const MAX_TRACKED_PORTS: usize = 256;

/// The port a socket's bytes belong to, and which end of the connection it is.
///
/// A socket whose local port is one this machine listens on is inbound, and
/// that local port names the service. Anything else is outbound, where the
/// local port is an ephemeral number the kernel picked and only the peer's
/// port carries meaning — charging it to 443 is what makes the row readable
/// as "HTTPS".
///
/// `None` means the socket carries no bytes of its own to charge: it is the
/// client end of a loopback connection to a local service, and the serving
/// socket on the other end already reports the very same traffic.
pub(super) fn charged_port(
    sample: &SocketSample,
    listening: &HashSet<u16>,
) -> Option<(u16, PortSide)> {
    if listening.contains(&sample.local_port) {
        return Some((sample.local_port, PortSide::Local));
    }
    if sample.remote_is_loopback && listening.contains(&sample.remote_port) {
        return None;
    }
    Some((sample.remote_port, PortSide::Remote))
}

/// Bytes a socket added since the previous refresh.
///
/// A socket seen for the first time contributes everything it has, so a
/// connection that opened and closed between two refreshes still lands on its
/// port in full. A counter that went backwards means the kernel handed the
/// inode to a new socket, and the current value is that new socket's whole
/// history.
pub(super) fn counter_delta(previous: u64, current: u64) -> u64 {
    if current < previous {
        return current;
    }
    current - previous
}

/// Running per-port byte totals across refreshes.
///
/// The kernel's counters live and die with each socket, so the totals are
/// accumulated here instead: every refresh folds each socket's delta into its
/// port and then forgets the sockets that closed.
#[derive(Default)]
pub(super) struct PortTrafficTracker {
    /// Counters last seen per socket inode, to turn the kernel's cumulative
    /// values into deltas.
    seen: HashMap<u64, (u64, u64)>,
    /// Bytes charged to each port since this tracker started.
    charged: HashMap<(u16, PortSide), (u64, u64)>,
    /// The same totals one refresh ago, which is what the rates divide.
    previous: HashMap<(u16, PortSide), (u64, u64)>,
    /// rx/tx rate rings per port, for the sparklines.
    history: HashMap<(u16, PortSide), (VecDeque<f32>, VecDeque<f32>)>,
}

impl PortTrafficTracker {
    pub(super) fn observe(
        &mut self,
        samples: &HashMap<u64, SocketSample>,
        listening: &HashSet<u16>,
        elapsed: f32,
    ) -> Vec<PortTraffic> {
        let mut open = HashMap::new();
        let mut still_open = HashMap::with_capacity(samples.len());
        for sample in samples.values() {
            let Some(key) = charged_port(sample, listening) else {
                continue;
            };
            let (prev_rx, prev_tx) = self.seen.get(&sample.inode).copied().unwrap_or((0, 0));
            let total = self.charged.entry(key).or_insert((0, 0));
            total.0 = total
                .0
                .saturating_add(counter_delta(prev_rx, sample.rx_bytes));
            total.1 = total
                .1
                .saturating_add(counter_delta(prev_tx, sample.tx_bytes));
            still_open.insert(sample.inode, (sample.rx_bytes, sample.tx_bytes));
            *open.entry(key).or_insert(0u32) += 1;
        }
        self.seen = still_open;

        let mut rows: Vec<PortTraffic> = Vec::with_capacity(self.charged.len());
        for (&key, &(rx_bytes, tx_bytes)) in &self.charged {
            let (prev_rx, prev_tx) = self.previous.get(&key).copied().unwrap_or((0, 0));
            let rx_bps = rate(rx_bytes.saturating_sub(prev_rx), elapsed);
            let tx_bps = rate(tx_bytes.saturating_sub(prev_tx), elapsed);
            let (rx_hist, tx_hist) = self
                .history
                .entry(key)
                .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
            push_capped(rx_hist, rx_bps as f32, HISTORY_SAMPLES);
            push_capped(tx_hist, tx_bps as f32, HISTORY_SAMPLES);
            let (port, side) = key;
            rows.push(PortTraffic {
                port,
                side,
                rx_bytes,
                tx_bytes,
                rx_bps,
                tx_bps,
                connections: open.get(&key).copied().unwrap_or(0),
                rx_hist: rx_hist.clone(),
                tx_hist: tx_hist.clone(),
            });
        }
        self.previous = self.charged.clone();
        self.forget_quietest(&open);

        rows.sort_by_key(|r| {
            std::cmp::Reverse((r.rx_bps + r.tx_bps, r.rx_bytes + r.tx_bytes, r.port))
        });
        rows.truncate(MAX_TRAFFIC_PORTS);
        rows
    }

    /// Drop the ports carrying the least, once too many are remembered.
    ///
    /// Anything with a socket open right now is kept regardless of how little
    /// it has moved, so a brand new connection is never forgotten mid-flight.
    fn forget_quietest(&mut self, open: &HashMap<(u16, PortSide), u32>) {
        if self.charged.len() <= MAX_TRACKED_PORTS {
            return;
        }
        let mut ranked: Vec<((u16, PortSide), u64)> = self
            .charged
            .iter()
            .map(|(&key, &(rx, tx))| (key, rx.saturating_add(tx)))
            .collect();
        ranked.sort_by_key(|&(_, total)| std::cmp::Reverse(total));
        let keep: HashSet<(u16, PortSide)> = ranked
            .into_iter()
            .take(MAX_TRACKED_PORTS)
            .map(|(key, _)| key)
            .chain(open.keys().copied())
            .collect();
        self.charged.retain(|key, _| keep.contains(key));
        self.previous.retain(|key, _| keep.contains(key));
        self.history.retain(|key, _| keep.contains(key));
    }
}

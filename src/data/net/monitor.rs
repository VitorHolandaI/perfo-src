//! Per-interface rate tracking across refreshes.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::data::disk::rate;

use super::parse::{link_state, netdev_from, tcp_stats_from, DevCounters};
use super::sockets::{listening_ports, proc_sockets, socket_inode_owners};
use super::{NetInfo, NetSnapshot, NetTotals};

pub struct NetMonitor {
    /// When the last refresh happened; drives the delta->rate conversion.
    last_refresh: Option<Instant>,
    prev: HashMap<String, DevCounters>,
    prev_retrans: u64,
    prev_proc_bytes: HashMap<u32, (u64, u64)>,
    /// Per-interface rx/tx rate rings for the sparklines.
    history: HashMap<String, (VecDeque<f32>, VecDeque<f32>)>,
    rx_history: VecDeque<f32>,
    tx_history: VecDeque<f32>,
    session_rx_bytes: u64,
    session_tx_bytes: u64,
    snapshot: NetSnapshot,
}
impl Default for NetMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl NetMonitor {
    pub fn new() -> Self {
        Self {
            last_refresh: None,
            prev: HashMap::new(),
            prev_retrans: 0,
            prev_proc_bytes: HashMap::new(),
            history: HashMap::new(),
            rx_history: VecDeque::new(),
            tx_history: VecDeque::new(),
            session_rx_bytes: 0,
            session_tx_bytes: 0,
            snapshot: NetSnapshot {
                ifaces: Vec::new(),
                totals: NetTotals::default(),
                rx_history: VecDeque::new(),
                tx_history: VecDeque::new(),
                proc_net: Vec::new(),
                listening: Vec::new(),
            },
        }
    }

    pub fn refresh(&mut self) {
        self.refresh_with_details(true, true);
    }

    /// Per-interface rates for this refresh, newest counters against the last
    /// ones, sorted with the busiest link first.
    ///
    /// Also advances the session totals and each interface's sparkline ring,
    /// and forgets the rings of interfaces that have gone away.
    fn interface_rates(
        &mut self,
        cur: &HashMap<String, DevCounters>,
        elapsed: f32,
    ) -> Vec<NetInfo> {
        let mut ifaces = Vec::new();
        for (name, c) in cur {
            // An interface with no previous sample has no delta to report, so
            // it only appears from the second refresh on.
            let Some(p) = self.prev.get(name) else {
                continue;
            };
            let d = |a: u64, b: u64| b.saturating_sub(a);
            let (link_mbps, link_up) = link_state(name);
            let rx_delta = d(p.0, c.0);
            let tx_delta = d(p.4, c.4);
            self.session_rx_bytes = self.session_rx_bytes.saturating_add(rx_delta);
            self.session_tx_bytes = self.session_tx_bytes.saturating_add(tx_delta);
            let rx_bps = rate(rx_delta, elapsed);
            let tx_bps = rate(tx_delta, elapsed);
            let (rq, tq) = self
                .history
                .entry(name.clone())
                .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
            push_capped(rq, rx_bps as f32, crate::data::disk::HISTORY_SAMPLES);
            push_capped(tq, tx_bps as f32, crate::data::disk::HISTORY_SAMPLES);
            ifaces.push(NetInfo {
                name: name.clone(),
                rx_bps,
                tx_bps,
                rx_pps: rate(d(p.1, c.1), elapsed),
                tx_pps: rate(d(p.5, c.5), elapsed),
                rx_errs_s: rate(d(p.2, c.2), elapsed),
                tx_errs_s: rate(d(p.6, c.6), elapsed),
                rx_drops_s: rate(d(p.3, c.3), elapsed),
                tx_drops_s: rate(d(p.7, c.7), elapsed),
                link_mbps,
                link_up,
                total_rx_bytes: c.0,
                total_tx_bytes: c.4,
                rx_hist: rq.clone(),
                tx_hist: tq.clone(),
            });
        }
        self.history.retain(|name, _| cur.contains_key(name));
        ifaces.sort_by_key(|a| std::cmp::Reverse(a.rx_bps));
        ifaces
    }

    /// Retransmits per second and the number of established connections.
    fn tcp_rates(&mut self, elapsed: f32) -> (u64, u64) {
        let (retrans_total, established) = tcp_stats_from(
            &std::fs::read_to_string("/proc/net/snmp").unwrap_or_default(),
            &std::fs::read_to_string("/proc/net/tcp").unwrap_or_default(),
            &std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default(),
        );
        let retrans_s = rate(retrans_total.saturating_sub(self.prev_retrans), elapsed);
        self.prev_retrans = retrans_total;
        (retrans_s, established)
    }

    pub fn refresh_with_details(&mut self, processes: bool, listeners: bool) {
        let now = Instant::now();
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let raw = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let cur = netdev_from(&raw);

        let ifaces = self.interface_rates(&cur, elapsed);
        let (retrans_s, established) = self.tcp_rates(elapsed);

        let total_rx: u64 = ifaces.iter().map(|i| i.rx_bps).sum();
        let total_tx: u64 = ifaces.iter().map(|i| i.tx_bps).sum();
        push_capped(
            &mut self.rx_history,
            total_rx as f32,
            crate::data::disk::HISTORY_SAMPLES,
        );
        push_capped(
            &mut self.tx_history,
            total_tx as f32,
            crate::data::disk::HISTORY_SAMPLES,
        );
        // One walk of /proc/<pid>/fd feeds both the per-process counters and
        // the listening-port table; it is the most expensive thing here.
        let owners = if processes || listeners {
            socket_inode_owners()
        } else {
            HashMap::new()
        };
        let proc_net = if processes {
            proc_sockets(&mut self.prev_proc_bytes, elapsed, &owners)
        } else {
            self.prev_proc_bytes.clear();
            Vec::new()
        };
        self.snapshot = NetSnapshot {
            totals: NetTotals {
                rx_bps: total_rx,
                tx_bps: total_tx,
                session_rx_bytes: self.session_rx_bytes,
                session_tx_bytes: self.session_tx_bytes,
                tcp_retrans_s: retrans_s,
                tcp_established: established,
            },
            ifaces,
            rx_history: self.rx_history.clone(),
            tx_history: self.tx_history.clone(),
            proc_net,
            listening: if listeners {
                listening_ports(&owners)
            } else {
                Vec::new()
            },
        };
        self.prev = cur;
        self.last_refresh = Some(now);
    }

    pub fn snapshot(&self) -> NetSnapshot {
        self.snapshot.clone()
    }
}

/// Push into a capped ring buffer.
fn push_capped(q: &mut VecDeque<f32>, v: f32, cap: usize) {
    q.push_back(v);
    if q.len() > cap {
        q.pop_front();
    }
}

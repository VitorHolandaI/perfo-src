//! Per-interface rate tracking across refreshes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use crate::data::disk::rate;

use super::inet_diag::tcp_socket_samples;
use super::parse::{link_state, netdev_from, tcp_stats_from, DevCounters};
use super::ports::PortTrafficTracker;
use super::push_capped;
use super::sockets::{listening_ports, proc_sockets, socket_inode_owners};
use super::{NetInfo, NetSnapshot, NetTotals};

pub struct NetMonitor {
    /// When the last refresh happened; drives the delta->rate conversion.
    last_refresh: Option<Instant>,
    prev: HashMap<String, DevCounters>,
    /// Counter values when this session started, subtracted out so the loss
    /// figures describe this run of the binary and not everything since boot.
    /// Cleared by `reset_loss`, which re-baselines on the next refresh.
    loss_baseline: HashMap<String, DevCounters>,
    prev_retrans: u64,
    prev_proc_bytes: HashMap<u32, (u64, u64)>,
    /// Per-port byte totals, which have to outlive the sockets that earned them.
    port_traffic: PortTrafficTracker,
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
            loss_baseline: HashMap::new(),
            prev_retrans: 0,
            prev_proc_bytes: HashMap::new(),
            port_traffic: PortTrafficTracker::default(),
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
                unowned_sockets: 0,
                ports: Vec::new(),
            },
        }
    }

    pub fn refresh(&mut self) {
        self.refresh_with_details(true, true);
    }

    /// This interface's baseline, seeded on first sight and moved forward
    /// when the kernel's counters restart.
    ///
    /// An interface that goes down and comes back up starts its counters at
    /// zero, which would make every subtraction against the old baseline
    /// underflow. The new, lower values become the baseline instead.
    fn baseline_for(&mut self, name: &str, cur: &DevCounters) -> DevCounters {
        let base = self
            .loss_baseline
            .entry(name.to_string())
            .or_insert_with(|| *cur);
        if counters_restarted(base, cur) {
            *base = *cur;
        }
        *base
    }

    /// Zero the loss figures from here on, so what the viewer reads next
    /// describes only what happened after they asked for it.
    pub fn reset_loss(&mut self) {
        self.loss_baseline.clear();
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
            let base = self.baseline_for(name, c);
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
                // Counts, not rates: dividing a single drop by the elapsed
                // second truncated it to zero and lost the event entirely.
                rx_errs: d(p.2, c.2),
                tx_errs: d(p.6, c.6),
                rx_drops: d(p.3, c.3),
                tx_drops: d(p.7, c.7),
                link_mbps,
                link_up,
                total_rx_bytes: c.0,
                total_tx_bytes: c.4,
                // `counters_restarted` already keeps the baseline at or below
                // the current values; saturating anyway so a future change to
                // that guard cannot turn into a wrapped u64 in release mode.
                session_rx_packets: c.1.saturating_sub(base.1),
                session_tx_packets: c.5.saturating_sub(base.5),
                session_rx_errs: c.2.saturating_sub(base.2),
                session_tx_errs: c.6.saturating_sub(base.6),
                session_rx_drops: c.3.saturating_sub(base.3),
                session_tx_drops: c.7.saturating_sub(base.7),
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
        // One netlink dump feeds the per-process counters and the per-port
        // table; both need every socket's bytes, so it is taken once.
        let samples = if processes || listeners {
            tcp_socket_samples()
        } else {
            HashMap::new()
        };
        let (proc_net, unowned_sockets) = if processes {
            proc_sockets(&mut self.prev_proc_bytes, elapsed, &owners, &samples)
        } else {
            self.prev_proc_bytes.clear();
            (Vec::new(), 0)
        };
        let listening = if listeners {
            listening_ports(&owners)
        } else {
            Vec::new()
        };
        let ports = if listeners {
            let served: HashSet<u16> = listening.iter().map(|l| l.port).collect();
            self.port_traffic.observe(&samples, &served, elapsed)
        } else {
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
            listening,
            ports,
            unowned_sockets,
        };
        self.prev = cur;
        self.last_refresh = Some(now);
    }

    pub fn snapshot(&self) -> NetSnapshot {
        self.snapshot.clone()
    }
}

/// Whether any counter this monitor subtracts went backwards, which only
/// happens when the interface was torn down and brought back.
fn counters_restarted(base: &DevCounters, cur: &DevCounters) -> bool {
    cur.1 < base.1
        || cur.2 < base.2
        || cur.3 < base.3
        || cur.5 < base.5
        || cur.6 < base.6
        || cur.7 < base.7
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `/proc/net/dev` row with only the columns these tests care about.
    fn counters(
        rx_packets: u64,
        rx_errs: u64,
        rx_drop: u64,
        tx_packets: u64,
        tx_errs: u64,
        tx_drop: u64,
    ) -> DevCounters {
        (
            0, rx_packets, rx_errs, rx_drop, 0, tx_packets, tx_errs, tx_drop,
        )
    }

    /// A monitor that has already seen `prev` for wg0, so the next sample
    /// produces deltas instead of being swallowed as the seeding refresh.
    fn seeded(prev: DevCounters) -> NetMonitor {
        let mut monitor = NetMonitor::new();
        monitor.prev.insert("wg0".to_string(), prev);
        monitor
    }

    fn sample(monitor: &mut NetMonitor, cur: DevCounters, elapsed: f32) -> NetInfo {
        let mut map = HashMap::new();
        map.insert("wg0".to_string(), cur);
        monitor.interface_rates(&map, elapsed).remove(0)
    }

    #[test]
    fn loss_from_before_the_binary_started_is_not_this_session() {
        // wg0 carrying 944 errors from hours ago. Nothing new happens, so the
        // session has seen no loss at all and must not inherit the backlog.
        let boot = counters(0, 0, 0, 7813, 944, 0);
        let mut monitor = seeded(boot);
        let iface = sample(&mut monitor, boot, 1.0);
        assert_eq!(iface.tx_errs, 0, "nothing new happened this refresh");
        assert_eq!(iface.session_tx_errs, 0, "944 of those predate the session");
        assert_eq!(iface.tx_loss_pct(), 0.0);
    }

    #[test]
    fn session_loss_accumulates_from_the_baseline() {
        // 944 on the kernel's counter when perfo opened, then 6 more errors
        // arrive across two refreshes. The session has seen 6, not 950.
        let mut monitor = seeded(counters(0, 0, 0, 7813, 944, 0));
        sample(&mut monitor, counters(0, 0, 0, 7813, 944, 0), 1.0);
        monitor
            .prev
            .insert("wg0".to_string(), counters(0, 0, 0, 7813, 944, 0));
        sample(&mut monitor, counters(0, 0, 0, 7815, 948, 0), 1.0);
        monitor
            .prev
            .insert("wg0".to_string(), counters(0, 0, 0, 7815, 948, 0));
        let iface = sample(&mut monitor, counters(0, 0, 0, 7817, 950, 0), 1.0);

        assert_eq!(iface.tx_errs, 2, "two arrived in this refresh alone");
        assert_eq!(iface.session_tx_errs, 6, "six since perfo opened");
        assert_eq!(iface.session_tx_packets, 4);
    }

    #[test]
    fn reset_loss_zeroes_the_session_without_touching_the_kernel() {
        let mut monitor = seeded(counters(0, 0, 0, 7813, 944, 0));
        sample(&mut monitor, counters(0, 0, 0, 7813, 944, 0), 1.0);
        monitor
            .prev
            .insert("wg0".to_string(), counters(0, 0, 0, 7813, 944, 0));
        let before = sample(&mut monitor, counters(0, 0, 0, 7820, 960, 0), 1.0);
        assert_eq!(before.session_tx_errs, 16);

        monitor.reset_loss();
        let after = sample(&mut monitor, counters(0, 0, 0, 7820, 960, 0), 1.0);
        assert_eq!(
            after.session_tx_errs, 0,
            "the viewer asked for a clean slate"
        );
    }

    #[test]
    fn a_lone_drop_is_not_rounded_away() {
        // As a rate this was 1/1.4 truncated to 0, so isolated drops never
        // showed up at all.
        let mut monitor = seeded(counters(100, 0, 0, 0, 0, 0));
        let iface = sample(&mut monitor, counters(100, 0, 1, 0, 0, 0), 1.4);
        assert_eq!(iface.rx_drops, 1);
    }

    #[test]
    fn loss_percent_weighs_drops_and_errors_against_everything_handled() {
        let lossy = NetInfo {
            session_rx_packets: 99,
            session_rx_drops: 1,
            ..NetInfo::default()
        };
        assert_eq!(lossy.rx_loss_pct(), 1.0);

        // Errors are loss too. wg0 on this machine carries 944 of them with
        // no drops at all, so a drops-only ratio would call it clean.
        let erroring = NetInfo {
            session_tx_packets: 7813,
            session_tx_errs: 944,
            ..NetInfo::default()
        };
        assert!(
            (erroring.tx_loss_pct() - 10.78).abs() < 0.01,
            "got {}",
            erroring.tx_loss_pct()
        );
    }

    #[test]
    fn loss_percent_is_zero_on_an_interface_that_moved_nothing() {
        assert_eq!(NetInfo::default().rx_loss_pct(), 0.0);
        assert_eq!(NetInfo::default().tx_loss_pct(), 0.0);
    }

    #[test]
    fn an_interface_that_restarts_its_counters_rebaselines_instead_of_underflowing() {
        // The link went down and came back, so cur is below both prev and the
        // baseline. Subtracting the old baseline would wrap around u64.
        let mut monitor = seeded(counters(5_000, 0, 40, 0, 0, 0));
        monitor
            .loss_baseline
            .insert("wg0".to_string(), counters(4_000, 0, 30, 0, 0, 0));
        let iface = sample(&mut monitor, counters(12, 0, 0, 0, 0, 0), 1.0);
        assert_eq!(iface.rx_drops, 0, "a restart is not a burst of loss");
        assert_eq!(iface.session_rx_drops, 0);
        assert_eq!(iface.session_rx_packets, 0);
    }
}

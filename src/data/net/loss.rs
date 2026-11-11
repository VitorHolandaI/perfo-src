//! Packet loss over a window that starts when the reader does.
//!
//! The kernel's columns in `/proc/net/dev` run from boot and keep hours of
//! history nobody is looking at any more: an idle bridge that discarded every
//! frame days ago sits at 100% for as long as the host is up. Everything here
//! is relative to a baseline captured when an interface is first seen.

use std::collections::HashMap;

use serde::Serialize;

use super::parse::{netdev_from, DevCounters};

/// Counter values an interface had when its window opened.
///
/// Shared by `NetMonitor`, which reports loss alongside everything else, and
/// by `NetLossMonitor`, which reports only loss. The rule for a restarted
/// interface has to be identical in both or the same link reads differently
/// in the TUI and in the widget.
#[derive(Default)]
pub(super) struct LossBaseline {
    base: HashMap<String, DevCounters>,
}

impl LossBaseline {
    /// This interface's baseline, seeded on first sight and moved forward when
    /// the kernel's counters restart.
    ///
    /// An interface that goes down and comes back up starts its counters at
    /// zero, which would make every subtraction against the old baseline
    /// underflow. The new, lower values become the baseline instead.
    pub(super) fn of(&mut self, name: &str, cur: &DevCounters) -> DevCounters {
        let base = self.base.entry(name.to_string()).or_insert_with(|| *cur);
        if counters_restarted(base, cur) {
            *base = *cur;
        }
        *base
    }

    /// Open a new window from the next sample on.
    pub(super) fn clear(&mut self) {
        self.base.clear();
    }
}

/// Whether any counter a caller subtracts went backwards, which only happens
/// when the interface was torn down and brought back.
fn counters_restarted(base: &DevCounters, cur: &DevCounters) -> bool {
    cur.1 < base.1
        || cur.2 < base.2
        || cur.3 < base.3
        || cur.5 < base.5
        || cur.6 < base.6
        || cur.7 < base.7
}

/// One interface's loss over the current window.
#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct InterfaceLoss {
    pub name: String,
    /// Packets the interface carried in both directions.
    pub packets: u64,
    /// Errors plus discards in both directions. Both count: a link can pile
    /// up thousands of one and none of the other.
    pub lost: u64,
    /// `lost / (packets + lost)`, in percent.
    pub loss_pct: f32,
}

/// Packet loss alone, for the always-on widget collector.
///
/// Reads one 3 KB file per refresh and nothing else. The full `NetMonitor`
/// also dumps sockets over netlink and walks `/proc/<pid>/fd`, which is far
/// too much to pay for continuously just to know whether a link is dropping.
#[derive(Default)]
pub struct NetLossMonitor {
    baseline: LossBaseline,
}

impl NetLossMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loss per interface, newest sample against the window's baseline.
    ///
    /// Only interfaces that carried at least one packet are reported. A bridge
    /// with no peer attached discards every frame it is handed and is honestly
    /// at 100%, but a list led by eleven idle docker bridges buries the one
    /// link that is really bleeding.
    pub fn refresh(&mut self) -> Vec<InterfaceLoss> {
        let raw = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
        self.losses_from(&raw)
    }

    fn losses_from(&mut self, raw: &str) -> Vec<InterfaceLoss> {
        let mut out: Vec<InterfaceLoss> = netdev_from(raw)
            .iter()
            .filter_map(|(name, cur)| {
                let base = self.baseline.of(name, cur);
                let packets = cur.1.saturating_sub(base.1) + cur.5.saturating_sub(base.5);
                if packets == 0 {
                    return None;
                }
                let lost = cur.2.saturating_sub(base.2)
                    + cur.3.saturating_sub(base.3)
                    + cur.6.saturating_sub(base.6)
                    + cur.7.saturating_sub(base.7);
                Some(InterfaceLoss {
                    name: name.clone(),
                    packets,
                    lost,
                    loss_pct: loss_pct(packets, lost),
                })
            })
            .collect();
        // Worst first: the reason to look at this list is to find the bad link.
        out.sort_by(|a, b| {
            b.loss_pct
                .total_cmp(&a.loss_pct)
                .then_with(|| b.packets.cmp(&a.packets))
        });
        out
    }
}

/// `lost / (delivered + lost)`, in percent.
///
/// /proc/net/dev counts delivered packets and lost ones in separate columns,
/// so the lost ones are added back to get a denominator that covers
/// everything the interface was handed.
pub(super) fn loss_pct(delivered: u64, lost: u64) -> f32 {
    let handled = delivered.saturating_add(lost);
    if handled == 0 {
        return 0.0;
    }
    lost as f32 * 100.0 / handled as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two `/proc/net/dev` rows: a real link and a bridge with no peer.
    fn dev(wg_packets: u64, wg_errs: u64, br_drops: u64) -> String {
        format!(
            "Inter-|   Receive                        |  Transmit\n\
             face |bytes packets errs drop fifo frame compressed multicast|bytes ...\n\
             \x20   wg0: 0 0 0 0 0 0 0 0 0 {wg_packets} {wg_errs} 0 0 0 0 0\n\
             virbr0: 0 0 0 0 0 0 0 0 0 0 0 {br_drops} 0 0 0 0\n"
        )
    }

    #[test]
    fn the_window_opens_at_the_first_sample() {
        // 944 errors already on the counter when the collector starts.
        let mut monitor = NetLossMonitor::new();
        assert_eq!(monitor.losses_from(&dev(7813, 944, 718)), vec![]);
    }

    #[test]
    fn loss_accumulates_from_the_baseline_not_from_boot() {
        let mut monitor = NetLossMonitor::new();
        monitor.losses_from(&dev(7813, 944, 718));
        let losses = monitor.losses_from(&dev(7913, 950, 730));

        assert_eq!(
            losses.len(),
            1,
            "the peerless bridge carried nothing: {losses:?}"
        );
        let wg0 = &losses[0];
        assert_eq!(wg0.name, "wg0");
        assert_eq!(wg0.packets, 100, "100 since the window opened, not 7913");
        assert_eq!(wg0.lost, 6, "6 since the window opened, not 950");
        assert!((wg0.loss_pct - 5.66).abs() < 0.01, "got {}", wg0.loss_pct);
    }

    #[test]
    fn a_bridge_that_starts_carrying_traffic_shows_up_with_its_loss() {
        let mut monitor = NetLossMonitor::new();
        monitor.losses_from(&dev(0, 0, 718));
        // Same sample, except virbr0 now forwarded 9 frames and dropped 1.
        let raw =
            dev(0, 0, 719).replace("virbr0: 0 0 0 0 0 0 0 0 0 0", "virbr0: 0 0 0 0 0 0 0 0 0 9");
        let losses = monitor.losses_from(&raw);
        assert_eq!(losses.len(), 1);
        assert_eq!(losses[0].name, "virbr0");
        assert_eq!((losses[0].packets, losses[0].lost), (9, 1));
    }

    #[test]
    fn the_worst_link_is_listed_first() {
        let mut monitor = NetLossMonitor::new();
        monitor.losses_from(&dev(0, 0, 0));
        let raw = dev(1_000, 1, 0).replace(
            "virbr0: 0 0 0 0 0 0 0 0 0 0 0 0",
            "virbr0: 0 0 0 0 0 0 0 0 0 100 0 50",
        );
        let losses = monitor.losses_from(&raw);
        assert_eq!(
            losses.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["virbr0", "wg0"]
        );
    }

    #[test]
    fn an_interface_that_restarts_rebaselines_instead_of_underflowing() {
        let mut monitor = NetLossMonitor::new();
        monitor.losses_from(&dev(7813, 944, 0));
        // The link went down and came back: its counters restart at zero.
        let losses = monitor.losses_from(&dev(12, 0, 0));
        assert_eq!(losses, vec![], "a restart is not a burst of loss");
    }

    #[test]
    fn clearing_the_baseline_opens_a_fresh_window() {
        let mut monitor = NetLossMonitor::new();
        monitor.losses_from(&dev(0, 0, 0));
        assert_eq!(monitor.losses_from(&dev(500, 10, 0)).len(), 1);
        monitor.baseline.clear();
        monitor.losses_from(&dev(500, 10, 0));
        assert_eq!(monitor.losses_from(&dev(500, 10, 0)), vec![]);
    }
}

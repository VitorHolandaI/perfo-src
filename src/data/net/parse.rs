//! Turning the /proc/net text files into counters.

use std::collections::HashMap;

use super::sockets::TCP_ESTABLISHED;

const DEV_FIELDS: usize = 16;
/// Minimum columns a line must have (tx_bytes is column 9, tx_drop 12).
const DEV_MIN_FIELDS: usize = 12;

/// Per-interface network counters from /proc/net/dev.
///
/// Line layout (after the name): rx_bytes rx_packets rx_errs rx_drop
/// rx_fifo rx_frame rx_compressed rx_multicast tx_bytes tx_packets
/// tx_errs tx_drop tx_fifo tx_colls tx_carrier tx_compressed.
pub(super) type DevCounters = (u64, u64, u64, u64, u64, u64, u64, u64);

pub(super) fn netdev_from(raw: &str) -> HashMap<String, DevCounters> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let (name, rest) = match line.split_once(':') {
            Some((n, r)) => (n.trim().to_string(), r),
            None => continue,
        };
        let mut f = rest.split_whitespace();
        let mut n = [0u64; DEV_FIELDS];
        let mut count = 0usize;
        for slot in n.iter_mut() {
            match f.next().and_then(|v| v.parse().ok()) {
                Some(v) => {
                    *slot = v;
                    count += 1;
                }
                None => break,
            }
        }
        if count < DEV_MIN_FIELDS {
            continue;
        }
        out.insert(
            name,
            (
                n[0],  // rx_bytes
                n[1],  // rx_packets
                n[2],  // rx_errs
                n[3],  // rx_drop
                n[8],  // tx_bytes
                n[9],  // tx_packets
                n[10], // tx_errs
                n[11], // tx_drop
            ),
        );
    }
    out
}

/// (tcp_retrans_total, tcp_established) from /proc/net/snmp and TCP tables.
pub(super) fn tcp_stats_from(snmp_raw: &str, tcp_raw: &str, tcp6_raw: &str) -> (u64, u64) {
    let mut retrans = 0u64;
    let mut in_snmp = false;
    for line in snmp_raw.lines() {
        // Header first ("Tcp: RtoAlgorithm ..."), values second ("Tcp: 1 ...").
        if line.starts_with("Tcp:") {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if in_snmp {
                // RetransSegs is field 13 (1-based) after "Tcp:" -> token 12.
                if let Some(v) = fields.get(12).and_then(|s| s.parse().ok()) {
                    retrans = v;
                }
                break;
            }
            in_snmp = true;
        }
    }
    let established = established_from(tcp_raw) + established_from(tcp6_raw);
    (retrans, established)
}

pub(super) fn established_from(raw: &str) -> u64 {
    raw.lines()
        .filter(|line| line.split_whitespace().nth(3) == Some(TCP_ESTABLISHED))
        .count() as u64
}

/// Link speed (Mbps) and carrier state from /sys/class/net/<iface>/.
/// Virtual interfaces (lo, veth) expose neither: both become None/false
/// with speed None signalling "no link concept".
pub(super) fn link_state(iface: &str) -> (Option<u64>, bool) {
    let base = format!("/sys/class/net/{iface}");
    let speed = std::fs::read_to_string(format!("{base}/speed"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0);
    let up = std::fs::read_to_string(format!("{base}/carrier"))
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    (speed, up)
}

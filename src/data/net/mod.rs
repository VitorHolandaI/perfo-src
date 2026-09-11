//! Network interface, socket and per-process traffic collection.

mod monitor;
mod parse;
mod sockets;

pub use monitor::NetMonitor;

use serde::Serialize;
use std::collections::VecDeque;

#[derive(Clone, Serialize)]
pub struct NetInfo {
    pub name: String,
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub rx_pps: u64,
    pub tx_pps: u64,
    pub rx_errs_s: u64,
    pub tx_errs_s: u64,
    pub rx_drops_s: u64,
    pub tx_drops_s: u64,
    pub link_mbps: Option<u64>,
    pub link_up: bool,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    /// rx/tx rate rings (newest last) for the network sparklines.
    #[serde(skip)]
    pub rx_hist: VecDeque<f32>,
    #[serde(skip)]
    pub tx_hist: VecDeque<f32>,
}

#[derive(Clone, Serialize, Default)]
pub struct NetTotals {
    pub rx_bps: u64,
    pub tx_bps: u64,
    /// Bytes received/sent since this monitor instance started.
    pub session_rx_bytes: u64,
    pub session_tx_bytes: u64,
    pub tcp_retrans_s: u64,
    pub tcp_established: u64,
}

#[derive(Clone, Default, Serialize)]
pub struct NetSnapshot {
    pub ifaces: Vec<NetInfo>,
    pub totals: NetTotals,
    /// Aggregate RX/TX rate history for the dashboard graph.
    pub rx_history: VecDeque<f32>,
    pub tx_history: VecDeque<f32>,
    /// Processes with open sockets (own + readable under yama).
    pub proc_net: Vec<ProcNet>,
    /// Listening ports with the serving process.
    pub listening: Vec<ListeningPort>,
}

/// Per-process socket counts (TCP established/listening, UDP) and network byte transfer.
#[derive(Clone, Serialize, Default, PartialEq, Eq, Debug)]
pub struct ProcNet {
    pub pid: u32,
    pub tcp_est: u32,
    pub tcp_listen: u32,
    pub udp: u32,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// A listening port with the process serving it.
#[derive(Clone, Serialize)]
pub struct ListeningPort {
    pub port: u16,
    pub proto: String,
    pub pid: u32,
    pub cmd: String,
}

#[cfg(test)]
mod tests {
    use super::parse::{netdev_from, tcp_stats_from};
    use super::sockets::{base_proto, socket_line, Sock};

    use crate::data::disk::rate;

    fn dev_line() -> &'static str {
        "  enp3s0: 1000 200 3 4 0 0 0 0 5000 100 2 6 0 0 0 0\n"
    }

    #[test]
    fn netdev_parses_columns() {
        let m = netdev_from(&format!(
            "Inter-|   Receive ...\nface |bytes ...\n{}",
            dev_line()
        ));
        let c = &m["enp3s0"];
        assert_eq!(c.0, 1000); // rx_bytes
        assert_eq!(c.1, 200); // rx_packets
        assert_eq!(c.2, 3); // rx_errs
        assert_eq!(c.3, 4); // rx_drop
        assert_eq!(c.4, 5000); // tx_bytes
        assert_eq!(c.5, 100); // tx_packets
        assert_eq!(c.6, 2); // tx_errs
        assert_eq!(c.7, 6); // tx_drop
    }

    #[test]
    fn netdev_skips_garbage() {
        let m = netdev_from("not an interface line\nlo: 1 2 3 4 5\n");
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn tcp_stats_parses_retrans_and_established() {
        let snmp = "Tcp: RtoAlgorithm RtoMin RtoMax MaxConn ActiveOpens PassiveOpens AttemptFails EstabResets CurrEstab InSegs OutSegs RetransSegs InErrs OutRsts\nTcp: 1 200 120000 -1 10 5 0 2 3 100 90 7 0 1\n";
        let tcp = "sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n  0: 0100007F:1F90 00000000:0000 01 00000000:00000000 00:00000000 00000000    30        0 1000 2 3\n  1: 0100007F:C350 00000000:0000 0A 00000000:00000000 00:00000000 00000000    30        0 1000 2 3\n  2: 0100007F:C351 00000000:0000 06 00000000:00000000 00:00000000 00000000    30        0 1000 2 3\n";
        let tcp6 = "sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n  0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 01 00000000:00000000 00:00000000 00000000    30        0 1001 2 3\n";
        let (retrans, established) = tcp_stats_from(snmp, tcp, tcp6);
        assert_eq!(retrans, 7);
        assert_eq!(established, 2);
    }

    #[test]
    fn socket_line_parses_state_and_inode() {
        let tcp_est = "  0: 0100007F:1F90 00000000:0000 01 00000000:00000000 00:00000000 00000000    30        0 12345 2 3\n";
        let tcp_listen = "  1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000    30        0 99999 1 3\n";
        let udp = "  2: 00000000:0035 00000000:0000 07 00000000:00000000 00:00000000 00000000    30        0 77777 1 3\n";
        assert_eq!(socket_line(tcp_est, false), Some((12345, Sock::TcpEst)));
        assert_eq!(
            socket_line(tcp_listen, false),
            Some((99999, Sock::TcpListen))
        );
        assert_eq!(socket_line(udp, true), Some((77777, Sock::Udp)));
        assert_eq!(socket_line("garbage", false), None);
    }

    #[test]
    fn rate_zero_elapsed_is_safe() {
        assert_eq!(rate(100, 0.0), 0);
        assert_eq!(rate(100, 2.0), 50);
    }

    #[test]
    fn base_proto_collapses_ipv6_suffix() {
        assert_eq!(base_proto("tcp"), "tcp");
        assert_eq!(base_proto("tcp6"), "tcp");
        assert_eq!(base_proto("udp6"), "udp");
    }
}

//! Network interface, socket and per-process traffic collection.

mod inet_diag;
mod monitor;
mod parse;
mod ports;
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
    /// Ports carrying TCP traffic, listening or not.
    pub ports: Vec<PortTraffic>,
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

/// Which side of a connection the charged port sits on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortSide {
    /// A port this machine listens on: it names the local service.
    Local,
    /// The peer's port on an outbound connection, where the local port is
    /// ephemeral and says nothing.
    #[default]
    Remote,
}

/// Bytes charged to one port. TCP only: the kernel keeps no cumulative byte
/// counter for UDP sockets, so QUIC, DNS and WireGuard are not counted here.
#[derive(Clone, Serialize, Default, PartialEq, Eq, Debug)]
pub struct PortTraffic {
    pub port: u16,
    pub side: PortSide,
    /// Bytes charged since the monitor started.
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_bps: u64,
    pub tx_bps: u64,
    /// Sockets currently open on this port.
    pub connections: u32,
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
    use super::inet_diag::{inet_diag_sample, SocketSample};
    use super::parse::{netdev_from, tcp_stats_from};
    use super::ports::{charged_port, counter_delta, PortTrafficTracker};
    use super::sockets::{base_proto, socket_line, Sock};
    use super::PortSide;

    use std::collections::{HashMap, HashSet};

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

    /// One inet_diag reply message: the 72-byte header followed by a
    /// TCP_INFO rtattr holding the byte counters.
    fn diag_msg(inode: u32, sport: u16, dport: u16, rx: u64, tx: u64) -> Vec<u8> {
        diag_msg_to(inode, sport, dport, rx, tx, [93, 184, 216, 34])
    }

    /// The same message, with the peer's IPv4 address spelled out.
    fn diag_msg_to(inode: u32, sport: u16, dport: u16, rx: u64, tx: u64, peer: [u8; 4]) -> Vec<u8> {
        let mut info = vec![0u8; 208];
        info[128..136].copy_from_slice(&rx.to_ne_bytes());
        info[200..208].copy_from_slice(&tx.to_ne_bytes());

        let mut msg = vec![0u8; 72];
        msg[0] = libc::AF_INET as u8;
        msg[24..28].copy_from_slice(&peer);
        msg[4..6].copy_from_slice(&sport.to_be_bytes());
        msg[6..8].copy_from_slice(&dport.to_be_bytes());
        msg[68..72].copy_from_slice(&inode.to_ne_bytes());

        msg.extend_from_slice(&((info.len() + 4) as u16).to_ne_bytes());
        msg.extend_from_slice(&2u16.to_ne_bytes()); // INET_DIAG_INFO
        msg.extend_from_slice(&info);
        msg
    }

    #[test]
    fn inet_diag_sample_reads_ports_as_big_endian() {
        let sample = inet_diag_sample(&diag_msg(4242, 54321, 443, 900, 100)).unwrap();
        assert_eq!(sample.inode, 4242);
        assert_eq!(sample.local_port, 54321);
        assert_eq!(sample.remote_port, 443);
        assert_eq!(sample.rx_bytes, 900);
        assert_eq!(sample.tx_bytes, 100);
    }

    #[test]
    fn inet_diag_sample_drops_unattached_sockets() {
        assert!(inet_diag_sample(&diag_msg(0, 1, 2, 3, 4)).is_none());
    }

    fn sample(inode: u64, local_port: u16, remote_port: u16, rx: u64, tx: u64) -> SocketSample {
        SocketSample {
            inode,
            local_port,
            remote_port,
            remote_is_loopback: false,
            rx_bytes: rx,
            tx_bytes: tx,
        }
    }

    #[test]
    fn charged_port_names_the_service_on_an_inbound_socket() {
        let listening = HashSet::from([22u16]);
        let inbound = sample(1, 22, 51000, 0, 0);
        assert_eq!(
            charged_port(&inbound, &listening),
            Some((22, PortSide::Local))
        );
    }

    #[test]
    fn charged_port_names_the_peer_on_an_outbound_socket() {
        let listening = HashSet::from([22u16]);
        let outbound = sample(1, 54321, 443, 0, 0);
        assert_eq!(
            charged_port(&outbound, &listening),
            Some((443, PortSide::Remote))
        );
    }

    #[test]
    fn inet_diag_sample_marks_a_loopback_peer() {
        let remote = inet_diag_sample(&diag_msg(1, 40000, 443, 0, 0)).unwrap();
        assert!(!remote.remote_is_loopback);
        let local = inet_diag_sample(&diag_msg_to(2, 40000, 8080, 0, 0, [127, 0, 0, 1])).unwrap();
        assert!(local.remote_is_loopback);
    }

    #[test]
    fn charged_port_skips_the_client_end_of_a_loopback_connection() {
        // Both ends of a loopback connection are in the dump. Charging both
        // would count the same bytes twice on the same port number.
        let listening = HashSet::from([8080u16]);
        let client = SocketSample {
            remote_is_loopback: true,
            ..sample(1, 54321, 8080, 900, 100)
        };
        assert_eq!(charged_port(&client, &listening), None);

        let server = sample(2, 8080, 54321, 100, 900);
        assert_eq!(
            charged_port(&server, &listening),
            Some((8080, PortSide::Local))
        );
    }

    #[test]
    fn charged_port_keeps_a_remote_peer_that_reuses_a_local_port_number() {
        // A remote host answering on 8080 is not the local 8080 service.
        let listening = HashSet::from([8080u16]);
        let outbound = sample(1, 54321, 8080, 0, 0);
        assert_eq!(
            charged_port(&outbound, &listening),
            Some((8080, PortSide::Remote))
        );
    }

    #[test]
    fn counter_delta_counts_a_first_sighting_in_full() {
        // A socket that opened and finished between two refreshes is still
        // seen once, and everything it moved has to land on its port.
        assert_eq!(counter_delta(0, 5000), 5000);
    }

    #[test]
    fn counter_delta_treats_a_backwards_counter_as_a_reused_inode() {
        assert_eq!(counter_delta(9000, 120), 120);
    }

    fn one(samples: Vec<SocketSample>) -> HashMap<u64, SocketSample> {
        samples.into_iter().map(|s| (s.inode, s)).collect()
    }

    #[test]
    fn tracker_accumulates_across_refreshes() {
        let mut tracker = PortTrafficTracker::default();
        let listening = HashSet::new();

        tracker.observe(
            &one(vec![sample(7, 40000, 443, 1000, 200)]),
            &listening,
            1.0,
        );
        let rows = tracker.observe(
            &one(vec![sample(7, 40000, 443, 2500, 600)]),
            &listening,
            1.0,
        );

        let row = rows.iter().find(|r| r.port == 443).expect("port 443 row");
        assert_eq!(row.rx_bytes, 2500);
        assert_eq!(row.tx_bytes, 600);
        assert_eq!(row.rx_bps, 1500);
        assert_eq!(row.tx_bps, 400);
        assert_eq!(row.connections, 1);
    }

    #[test]
    fn tracker_keeps_a_closed_socket_total_and_drops_its_connection() {
        let mut tracker = PortTrafficTracker::default();
        let listening = HashSet::new();

        tracker.observe(&one(vec![sample(7, 40000, 443, 3000, 0)]), &listening, 1.0);
        let rows = tracker.observe(&one(vec![]), &listening, 1.0);

        let row = rows.iter().find(|r| r.port == 443).expect("port 443 row");
        assert_eq!(row.rx_bytes, 3000);
        assert_eq!(row.rx_bps, 0);
        assert_eq!(row.connections, 0);
    }

    #[test]
    fn tracker_sums_every_socket_charged_to_the_same_port() {
        let mut tracker = PortTrafficTracker::default();
        let listening = HashSet::new();
        let rows = tracker.observe(
            &one(vec![
                sample(1, 40000, 443, 100, 10),
                sample(2, 40001, 443, 400, 40),
            ]),
            &listening,
            1.0,
        );

        let row = rows.iter().find(|r| r.port == 443).expect("port 443 row");
        assert_eq!(row.rx_bytes, 500);
        assert_eq!(row.tx_bytes, 50);
        assert_eq!(row.connections, 2);
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

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use serde::Serialize;

use super::disk::rate;

/// /proc/net/dev columns read per interface (16 = rx+tx pairs with fifo).
const DEV_FIELDS: usize = 16;
/// Minimum columns a line must have (tx_bytes is column 9, tx_drop 12).
const DEV_MIN_FIELDS: usize = 12;

/// Per-interface network counters from /proc/net/dev.
///
/// Line layout (after the name): rx_bytes rx_packets rx_errs rx_drop
/// rx_fifo rx_frame rx_compressed rx_multicast tx_bytes tx_packets
/// tx_errs tx_drop tx_fifo tx_colls tx_carrier tx_compressed.
type DevCounters = (u64, u64, u64, u64, u64, u64, u64, u64);

fn netdev_from(raw: &str) -> HashMap<String, DevCounters> {
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
fn tcp_stats_from(snmp_raw: &str, tcp_raw: &str, tcp6_raw: &str) -> (u64, u64) {
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

fn established_from(raw: &str) -> u64 {
    raw.lines()
        .filter(|line| line.split_whitespace().nth(3) == Some(TCP_ESTABLISHED))
        .count() as u64
}

/// Link speed (Mbps) and carrier state from /sys/class/net/<iface>/.
/// Virtual interfaces (lo, veth) expose neither: both become None/false
/// with speed None signalling "no link concept".
fn link_state(iface: &str) -> (Option<u64>, bool) {
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

#[derive(Clone, Serialize)]
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
        let now = Instant::now();
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let raw = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
        let cur = netdev_from(&raw);
        let mut ifaces = Vec::new();
        for (name, c) in &cur {
            if let Some(p) = self.prev.get(name) {
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
                push_capped(rq, rx_bps as f32, super::disk::HISTORY_SAMPLES);
                push_capped(tq, tx_bps as f32, super::disk::HISTORY_SAMPLES);
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
        }
        self.history.retain(|name, _| cur.contains_key(name));
        ifaces.sort_by_key(|a| std::cmp::Reverse(a.rx_bps));
        let (retrans_total, established) = tcp_stats_from(
            &std::fs::read_to_string("/proc/net/snmp").unwrap_or_default(),
            &std::fs::read_to_string("/proc/net/tcp").unwrap_or_default(),
            &std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default(),
        );
        let retrans_s = rate(retrans_total.saturating_sub(self.prev_retrans), elapsed);
        self.prev_retrans = retrans_total;
        let total_rx: u64 = ifaces.iter().map(|i| i.rx_bps).sum();
        let total_tx: u64 = ifaces.iter().map(|i| i.tx_bps).sum();
        push_capped(
            &mut self.rx_history,
            total_rx as f32,
            super::disk::HISTORY_SAMPLES,
        );
        push_capped(
            &mut self.tx_history,
            total_tx as f32,
            super::disk::HISTORY_SAMPLES,
        );
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
            proc_net: proc_sockets(&mut self.prev_proc_bytes, elapsed),
            listening: listening_ports(),
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

/// Socket class from the `/proc/net/{tcp,tcp6,udp,udp6}` state field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sock {
    TcpEst,
    TcpListen,
    TcpOther,
    Udp,
}

/// inode -> socket class, from the four /proc/net socket tables.
fn socket_inodes() -> HashMap<u64, Sock> {
    let mut out = HashMap::new();
    for (path, udp) in [
        ("/proc/net/tcp", false),
        ("/proc/net/tcp6", false),
        ("/proc/net/udp", true),
        ("/proc/net/udp6", true),
    ] {
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        for line in raw.lines().skip(1) {
            if let Some((inode, class)) = socket_line(line, udp) {
                out.insert(inode, class);
            }
        }
    }
    out
}

/// TCP socket-table state hex codes (from net/tcp.h).
const TCP_ESTABLISHED: &str = "01";
const TCP_LISTEN: &str = "0A";
const UDP_UNCONNECTED: &str = "07";

/// (inode, class) from one `/proc/net/tcp`/`udp` line, when parseable.
fn socket_line(line: &str, udp: bool) -> Option<(u64, Sock)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let st = *fields.get(3)?;
    // inode is field 9 (0-based) in both tcp and udp tables: sl local rem
    // st tx:rx tr retrnsmt uid timeout inode refs ptr.
    let inode = fields.get(9)?.parse::<u64>().ok()?;
    let class = if udp {
        Sock::Udp
    } else {
        match st {
            TCP_ESTABLISHED => Sock::TcpEst,
            TCP_LISTEN => Sock::TcpListen,
            _ => Sock::TcpOther,
        }
    };
    Some((inode, class))
}

/// Inode -> (rx_bytes, tx_bytes) from Netlink INET_DIAG TCP sockets.
fn tcp_socket_bytes() -> HashMap<u64, (u64, u64)> {
    let mut out = HashMap::new();
    for family in [libc::AF_INET as u8, libc::AF_INET6 as u8] {
        query_inet_diag(family, &mut out);
    }
    out
}

fn query_inet_diag(family: u8, out: &mut HashMap<u64, (u64, u64)>) {
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW,
            4, /* NETLINK_INET_DIAG */
        );
        if fd < 0 {
            return;
        }
        let tv = libc::timeval {
            tv_sec: 0,
            tv_usec: 100_000,
        };
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );

        let mut req = [0u8; 72];
        req[0..4].copy_from_slice(&72u32.to_ne_bytes());
        req[4..6].copy_from_slice(&20u16.to_ne_bytes());
        req[6..8].copy_from_slice(&0x301u16.to_ne_bytes());
        req[8..12].copy_from_slice(&1u32.to_ne_bytes());
        req[16] = family;
        req[17] = 6; // IPPROTO_TCP
        req[18] = 2; // INET_DIAG_INFO
        req[20..24].copy_from_slice(&0xffff_ffffu32.to_ne_bytes());

        let sent = libc::send(fd, req.as_ptr() as *const libc::c_void, req.len(), 0);
        if sent < 0 {
            libc::close(fd);
            return;
        }

        let mut buf = [0u8; 65536];
        loop {
            let n = libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0);
            if n <= 0 {
                break;
            }
            let n = n as usize;
            let mut offset = 0;
            let mut done = false;
            while offset + 16 <= n {
                let nl_len =
                    u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap_or_default())
                        as usize;
                let nl_type =
                    u16::from_ne_bytes(buf[offset + 4..offset + 6].try_into().unwrap_or_default());
                if nl_len < 16 || offset + nl_len > n || nl_type == 2 || nl_type == 3 {
                    done = true;
                    break;
                }
                let msg = &buf[offset + 16..offset + nl_len];
                if msg.len() >= 72 {
                    let inode =
                        u32::from_ne_bytes(msg[68..72].try_into().unwrap_or_default()) as u64;
                    let mut rta_offset = 72;
                    while rta_offset + 4 <= msg.len() {
                        let rta_len = u16::from_ne_bytes(
                            msg[rta_offset..rta_offset + 2]
                                .try_into()
                                .unwrap_or_default(),
                        ) as usize;
                        let rta_type = u16::from_ne_bytes(
                            msg[rta_offset + 2..rta_offset + 4]
                                .try_into()
                                .unwrap_or_default(),
                        );
                        if rta_len < 4 || rta_offset + rta_len > msg.len() {
                            break;
                        }
                        if rta_type == 2
                        /* INET_DIAG_INFO */
                        {
                            let info = &msg[rta_offset + 4..rta_offset + rta_len];
                            if info.len() >= 208 {
                                let rx = u64::from_ne_bytes(
                                    info[128..136].try_into().unwrap_or_default(),
                                );
                                let tx = u64::from_ne_bytes(
                                    info[200..208].try_into().unwrap_or_default(),
                                );
                                if inode > 0 {
                                    out.insert(inode, (rx, tx));
                                }
                            }
                        }
                        rta_offset += (rta_len + 3) & !3;
                    }
                }
                offset += (nl_len + 3) & !3;
            }
            if done {
                break;
            }
        }
        libc::close(fd);
    }
}

/// Processes with open sockets, resolved by scanning /proc/<pid>/fd for
/// `socket:[inode]` links. Only own (yama-visible) processes are readable.
fn proc_sockets(prev_proc_bytes: &mut HashMap<u32, (u64, u64)>, elapsed: f32) -> Vec<ProcNet> {
    let inodes = socket_inodes();
    let socket_bytes = tcp_socket_bytes();
    let mut per_pid: HashMap<u32, ProcNet> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for e in entries.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(link) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let s = link.to_string_lossy();
                let Some(inode) = s
                    .strip_prefix("socket:[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                let Some(class) = inodes.get(&inode) else {
                    continue;
                };
                let entry = per_pid.entry(pid).or_default();
                entry.pid = pid;
                match class {
                    Sock::TcpEst => entry.tcp_est += 1,
                    Sock::TcpListen => entry.tcp_listen += 1,
                    Sock::Udp => entry.udp += 1,
                    Sock::TcpOther => {}
                }
                if let Some(&(rx, tx)) = socket_bytes.get(&inode) {
                    entry.rx_bytes = entry.rx_bytes.saturating_add(rx);
                    entry.tx_bytes = entry.tx_bytes.saturating_add(tx);
                }
            }
        }
    }

    let mut cur_active = HashMap::new();
    for entry in per_pid.values_mut() {
        if let Some(&(prev_rx, prev_tx)) = prev_proc_bytes.get(&entry.pid) {
            let rx_delta = entry.rx_bytes.saturating_sub(prev_rx);
            let tx_delta = entry.tx_bytes.saturating_sub(prev_tx);
            entry.rx_bps = rate(rx_delta, elapsed);
            entry.tx_bps = rate(tx_delta, elapsed);
        }
        cur_active.insert(entry.pid, (entry.rx_bytes, entry.tx_bytes));
    }
    *prev_proc_bytes = cur_active;

    let mut list: Vec<ProcNet> = per_pid.into_values().collect();
    list.sort_by_key(|p| {
        std::cmp::Reverse(
            (p.rx_bps + p.tx_bps) * 1000
                + (p.rx_bytes + p.tx_bytes)
                + (p.tcp_est as u64 * 100)
                + (p.tcp_listen + p.udp) as u64,
        )
    });
    list.truncate(32);
    list
}

/// (inode, port, proto, uid) from listening entries across the four proc files.
fn listening_sockets() -> Vec<(u64, u16, String, u32)> {
    let mut out = Vec::new();
    for (path, proto) in [
        ("/proc/net/tcp", "tcp"),
        ("/proc/net/tcp6", "tcp6"),
        ("/proc/net/udp", "udp"),
        ("/proc/net/udp6", "udp6"),
    ] {
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        for line in raw.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            let st = fields[3];
            let is_listening = if proto.starts_with("udp") {
                st == UDP_UNCONNECTED
            } else {
                st == TCP_LISTEN
            };
            if !is_listening {
                continue;
            }
            let local = fields[1];
            let port_hex = local.split(':').nth(1).unwrap_or("0");
            let port = u16::from_str_radix(port_hex, 16).unwrap_or(0);
            if port == 0 {
                continue;
            }
            let inode = match fields[9].parse::<u64>() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let uid = fields
                .get(7)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            out.push((inode, port, proto.to_string(), uid));
        }
    }
    out
}

/// Discover listening ports, PID (when readable), and cmdline.
fn listening_ports() -> Vec<ListeningPort> {
    let sockets = listening_sockets();
    if sockets.is_empty() {
        return Vec::new();
    }
    // Build inode → (port, proto, uid) lookup.
    let mut inode_map: HashMap<u64, (u16, String, u32)> = HashMap::new();
    for (inode, port, proto, uid) in &sockets {
        inode_map
            .entry(*inode)
            .or_insert((*port, proto.clone(), *uid));
    }
    // Scan /proc/<pid>/fd for matching socket inodes.
    let mut pid_map: HashMap<u64, u32> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for e in entries.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(link) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let s = link.to_string_lossy();
                let Some(inode) = s
                    .strip_prefix("socket:[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                if inode_map.contains_key(&inode) {
                    pid_map.entry(inode).or_insert(pid);
                }
            }
        }
    }
    // Collapse IPv4/IPv6 duplicates into one port/protocol row.
    let mut by_key: HashMap<(u16, String), Vec<(u32, u32)>> = HashMap::new();
    for (inode, port, proto, uid) in &sockets {
        let pid = pid_map.get(inode).copied().unwrap_or(0);
        by_key
            .entry((*port, base_proto(proto)))
            .or_default()
            .push((pid, *uid));
    }
    let mut result: Vec<ListeningPort> = by_key
        .into_iter()
        .map(|((port, proto), owners)| {
            let best_pid = owners
                .iter()
                .map(|(pid, _)| *pid)
                .filter(|pid| *pid != 0)
                .max()
                .unwrap_or(0);
            let mut uids: Vec<u32> = owners.iter().map(|(_, uid)| *uid).collect();
            uids.sort_unstable();
            uids.dedup();
            let cmd = if best_pid != 0 {
                std::fs::read_to_string(format!("/proc/{best_pid}/cmdline"))
                    .unwrap_or_default()
                    .replace('\0', " ")
            } else {
                format!(
                    "(uid {})",
                    uids.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            ListeningPort {
                port,
                proto,
                pid: best_pid,
                cmd,
            }
        })
        .collect();
    result.sort_by_key(|p| p.port);
    result
}

fn base_proto(proto: &str) -> String {
    proto.strip_suffix('6').unwrap_or(proto).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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

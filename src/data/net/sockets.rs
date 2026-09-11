//! Per-process socket accounting, via /proc and the sock_diag netlink API.

use std::collections::HashMap;

/// Scores a process's socket activity: throughput dominates, and each
/// established connection adds a small tiebreak.
const RATE_WEIGHT: u64 = 1000;
const CONNECTION_WEIGHT: u64 = 100;
/// How many processes the socket list keeps.
const MAX_SOCKET_PROCESSES: usize = 32;
/// A /proc/net row shorter than this is truncated or a header.
const MIN_SOCK_FIELDS: usize = 10;
/// Ports are written in hex in /proc/net.
const PORT_RADIX: u32 = 16;

use crate::data::disk::rate;

use super::{ListeningPort, ProcNet};

/// Socket class from the `/proc/net/{tcp,tcp6,udp,udp6}` state field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Sock {
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
pub(super) const TCP_ESTABLISHED: &str = "01";
const TCP_LISTEN: &str = "0A";
const UDP_UNCONNECTED: &str = "07";

/// (inode, class) from one `/proc/net/tcp`/`udp` line, when parseable.
pub(super) fn socket_line(line: &str, udp: bool) -> Option<(u64, Sock)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let st = *fields.get(SOCK_STATE_FIELD)?;
    // inode is field 9 (0-based) in both tcp and udp tables: sl local rem
    // st tx:rx tr retrnsmt uid timeout inode refs ptr.
    let inode = fields.get(SOCK_INODE_FIELD)?.parse::<u64>().ok()?;
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

// Kernel ABI for the sock_diag netlink interface. These are offsets and tags
// from <linux/inet_diag.h>, <linux/netlink.h> and <linux/tcp.h>; they are
// spelled out here because the raw numbers cannot be checked without the
// headers open next to the code.
/// Column of the connection state in a /proc/net/{tcp,udp} row.
/// Protocol byte of the inet_diag request.
const IPPROTO_TCP: u8 = 6;

const SOCK_STATE_FIELD: usize = 3;
/// Column of the socket inode in that same row.
const SOCK_INODE_FIELD: usize = 9;
/// How long to wait for the kernel's dump before giving up.
const RECV_TIMEOUT_USEC: i64 = 100_000;
/// Every idiag_states bit set: dump sockets in any state.
const IDIAG_ALL_STATES: u32 = 0xffff_ffff;
/// Receive buffer for one netlink datagram.
const NL_RECV_BUF: usize = 65536;
/// NLMSG_ALIGNTO: netlink rounds every length up to this boundary.
const NL_ALIGN_TO: usize = 4;

const NETLINK_INET_DIAG: libc::c_int = 4;
const SOCK_DIAG_BY_FAMILY: u16 = 20;
/// NLM_F_REQUEST | NLM_F_DUMP
const NLM_FLAGS_REQUEST_DUMP: u16 = 0x301;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLMSG_HDRLEN: usize = 16;
const INET_DIAG_INFO: u16 = 2;
/// nlmsghdr (16) + inet_diag_req_v2 (56)
const INET_DIAG_REQ_LEN: usize = 72;
/// Every field of inet_diag_msg up to and including idiag_inode.
const INET_DIAG_MSG_LEN: usize = 72;
/// Byte range of idiag_inode inside inet_diag_msg.
const IDIAG_INODE: std::ops::Range<usize> = 68..72;
/// Byte ranges of tcpi_bytes_received and tcpi_bytes_sent inside tcp_info.
const TCPI_BYTES_RECEIVED: std::ops::Range<usize> = 128..136;
const TCPI_BYTES_SENT: std::ops::Range<usize> = 200..208;
/// Shortest tcp_info that still carries tcpi_bytes_sent.
const TCP_INFO_MIN_LEN: usize = TCPI_BYTES_SENT.end;
const RTATTR_HDRLEN: usize = 4;
/// NLMSG_ALIGN / RTA_ALIGN round up to a 4-byte boundary.
fn nl_align(len: usize) -> usize {
    (len + NL_ALIGN_TO - 1) & !(NL_ALIGN_TO - 1)
}

/// Opens a netlink socket and sends one SOCK_DIAG_BY_FAMILY dump request.
///
/// Returns the fd, already set to time out on receive, or None if the kernel
/// refused either step.
unsafe fn open_inet_diag(family: u8) -> Option<libc::c_int> {
    let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_INET_DIAG);
    if fd < 0 {
        return None;
    }
    let tv = libc::timeval {
        tv_sec: 0,
        tv_usec: RECV_TIMEOUT_USEC,
    };
    libc::setsockopt(
        fd,
        libc::SOL_SOCKET,
        libc::SO_RCVTIMEO,
        &tv as *const _ as *const libc::c_void,
        std::mem::size_of::<libc::timeval>() as libc::socklen_t,
    );

    let mut req = [0u8; INET_DIAG_REQ_LEN];
    req[0..4].copy_from_slice(&(INET_DIAG_REQ_LEN as u32).to_ne_bytes());
    req[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    req[6..8].copy_from_slice(&NLM_FLAGS_REQUEST_DUMP.to_ne_bytes());
    req[8..12].copy_from_slice(&1u32.to_ne_bytes());
    req[16] = family;
    req[17] = IPPROTO_TCP;
    req[18] = INET_DIAG_INFO as u8;
    req[20..24].copy_from_slice(&IDIAG_ALL_STATES.to_ne_bytes());

    let sent = libc::send(fd, req.as_ptr() as *const libc::c_void, req.len(), 0);
    if sent < 0 {
        libc::close(fd);
        return None;
    }
    Some(fd)
}

/// Walks the netlink reply, pulling each socket's byte counters into `out`.
unsafe fn read_inet_diag_reply(fd: libc::c_int, out: &mut HashMap<u64, (u64, u64)>) {
    let mut buf = [0u8; NL_RECV_BUF];
    loop {
        let n = libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0);
        if n <= 0 {
            break;
        }
        let n = n as usize;
        let mut offset = 0;
        let mut done = false;
        while offset + NLMSG_HDRLEN <= n {
            let nl_len =
                u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap_or_default()) as usize;
            let nl_type =
                u16::from_ne_bytes(buf[offset + 4..offset + 6].try_into().unwrap_or_default());
            if nl_len < NLMSG_HDRLEN
                || offset + nl_len > n
                || nl_type == NLMSG_ERROR
                || nl_type == NLMSG_DONE
            {
                done = true;
                break;
            }
            let msg = &buf[offset + NLMSG_HDRLEN..offset + nl_len];
            if msg.len() >= INET_DIAG_MSG_LEN {
                let inode =
                    u32::from_ne_bytes(msg[IDIAG_INODE].try_into().unwrap_or_default()) as u64;
                let mut rta_offset = INET_DIAG_MSG_LEN;
                while rta_offset + RTATTR_HDRLEN <= msg.len() {
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
                    if rta_len < RTATTR_HDRLEN || rta_offset + rta_len > msg.len() {
                        break;
                    }
                    if rta_type == INET_DIAG_INFO {
                        let info = &msg[rta_offset + RTATTR_HDRLEN..rta_offset + rta_len];
                        if info.len() >= TCP_INFO_MIN_LEN {
                            let rx = u64::from_ne_bytes(
                                info[TCPI_BYTES_RECEIVED].try_into().unwrap_or_default(),
                            );
                            let tx = u64::from_ne_bytes(
                                info[TCPI_BYTES_SENT].try_into().unwrap_or_default(),
                            );
                            if inode > 0 {
                                out.insert(inode, (rx, tx));
                            }
                        }
                    }
                    rta_offset += nl_align(rta_len);
                }
            }
            offset += nl_align(nl_len);
        }
        if done {
            break;
        }
    }
}

fn query_inet_diag(family: u8, out: &mut HashMap<u64, (u64, u64)>) {
    unsafe {
        let Some(fd) = open_inet_diag(family) else {
            return;
        };
        read_inet_diag_reply(fd, out);
        libc::close(fd);
    }
}

/// Processes with open sockets, resolved by scanning /proc/<pid>/fd for
/// `socket:[inode]` links. Only own (yama-visible) processes are readable.
pub(super) fn proc_sockets(
    prev_proc_bytes: &mut HashMap<u32, (u64, u64)>,
    elapsed: f32,
) -> Vec<ProcNet> {
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
            (p.rx_bps + p.tx_bps) * RATE_WEIGHT
                + (p.rx_bytes + p.tx_bytes)
                + (p.tcp_est as u64 * CONNECTION_WEIGHT)
                + (p.tcp_listen + p.udp) as u64,
        )
    });
    list.truncate(MAX_SOCKET_PROCESSES);
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
            if fields.len() < MIN_SOCK_FIELDS {
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
            let port = u16::from_str_radix(port_hex, PORT_RADIX).unwrap_or(0);
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
pub(super) fn listening_ports() -> Vec<ListeningPort> {
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

pub(super) fn base_proto(proto: &str) -> String {
    proto.strip_suffix('6').unwrap_or(proto).to_string()
}

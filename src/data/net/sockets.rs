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

use super::inet_diag::SocketSample;
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

/// Column of the connection state in a /proc/net/{tcp,udp} row.
const SOCK_STATE_FIELD: usize = 3;
/// Column of the socket inode in that same row.
const SOCK_INODE_FIELD: usize = 9;

/// Processes with open sockets, resolved by scanning /proc/<pid>/fd for
/// `socket:[inode]` links. Only own (yama-visible) processes are readable.
/// Maps every socket inode visible in /proc to the pid holding it.
///
/// Walking `/proc/<pid>/fd` and readlink-ing each entry is the most expensive
/// thing this collector does — on a busy desktop it is tens of thousands of
/// syscalls. Both the per-process counters and the listening-port table need
/// the same mapping, so the walk happens once and they share the result.
pub(super) fn socket_inode_owners() -> HashMap<u64, u32> {
    let mut owners = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return owners;
    };
    for e in entries.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let Some(inode) = link
                .to_str()
                .and_then(|s| s.strip_prefix("socket:["))
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|n| n.parse::<u64>().ok())
            else {
                continue;
            };
            owners.entry(inode).or_insert(pid);
        }
    }
    owners
}

pub(super) fn proc_sockets(
    prev_proc_bytes: &mut HashMap<u32, (u64, u64)>,
    elapsed: f32,
    owners: &HashMap<u64, u32>,
    samples: &HashMap<u64, SocketSample>,
) -> Vec<ProcNet> {
    let inodes = socket_inodes();
    let mut per_pid: HashMap<u32, ProcNet> = HashMap::new();
    for (&inode, &pid) in owners {
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
        if let Some(sample) = samples.get(&inode) {
            entry.rx_bytes = entry.rx_bytes.saturating_add(sample.rx_bytes);
            entry.tx_bytes = entry.tx_bytes.saturating_add(sample.tx_bytes);
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
pub(super) fn listening_ports(owners: &HashMap<u64, u32>) -> Vec<ListeningPort> {
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
    let pid_map: HashMap<u64, u32> = inode_map
        .keys()
        .filter_map(|inode| owners.get(inode).map(|pid| (*inode, *pid)))
        .collect();
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

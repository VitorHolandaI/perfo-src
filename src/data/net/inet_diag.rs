//! The sock_diag netlink dump: one round trip that yields every TCP socket's
//! ports, peer and byte counters.

use std::collections::HashMap;

/// One TCP socket as the kernel reports it: which ports it joins and how
/// many bytes have crossed it since it opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct SocketSample {
    pub(super) inode: u64,
    pub(super) local_port: u16,
    pub(super) remote_port: u16,
    /// Whether the peer is on this machine, which makes the connection show
    /// up twice in the dump: once per end.
    pub(super) remote_is_loopback: bool,
    pub(super) rx_bytes: u64,
    pub(super) tx_bytes: u64,
}

/// Inode -> socket sample from Netlink INET_DIAG TCP sockets.
///
/// One dump serves both the per-process counters and the per-port table, so
/// the caller takes it once per refresh and lends it to each.
pub(super) fn tcp_socket_samples() -> HashMap<u64, SocketSample> {
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
/// Protocol byte of the inet_diag request.
const IPPROTO_TCP: u8 = 6;

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
/// Byte ranges of idiag_sport and idiag_dport, the first two fields of the
/// inet_diag_sockid that follows the 4-byte message header. Both are __be16,
/// so they are network order even on a little-endian host.
const IDIAG_SPORT: std::ops::Range<usize> = 4..6;
const IDIAG_DPORT: std::ops::Range<usize> = 6..8;
/// Byte range of idiag_dst, the 16-byte peer address that follows idiag_src.
/// IPv4 uses only its first four bytes.
const IDIAG_DST: std::ops::Range<usize> = 24..40;
/// Offset of idiag_family, which says how to read that address.
const IDIAG_FAMILY: usize = 0;
/// First octet of 127.0.0.0/8.
const IPV4_LOOPBACK_PREFIX: u8 = 127;
/// The ::ffff:0:0/96 prefix that carries an IPv4 address inside IPv6.
const IPV4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff];

/// Whether an inet_diag address is a loopback one.
///
/// Both ends of a loopback connection appear in the dump as separate sockets,
/// so the caller needs to tell them apart to avoid counting the same bytes on
/// each side.
fn is_loopback(family: u8, addr: &[u8]) -> bool {
    if addr.len() < 16 {
        return false;
    }
    if family == libc::AF_INET as u8 {
        return addr[0] == IPV4_LOOPBACK_PREFIX;
    }
    if family != libc::AF_INET6 as u8 {
        return false;
    }
    if addr[..12] == IPV4_MAPPED_PREFIX {
        return addr[12] == IPV4_LOOPBACK_PREFIX;
    }
    addr[..15].iter().all(|b| *b == 0) && addr[15] == 1
}
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
/// Length of the complete netlink message at `offset`.
///
/// `None` ends the walk, for any of three reasons the caller has to tell
/// apart: the buffer ran out mid-message, the length field is malformed, or
/// the kernel sent NLMSG_DONE / NLMSG_ERROR.
fn nlmsg_payload_len(buf: &[u8], offset: usize, filled: usize) -> Option<usize> {
    if offset + NLMSG_HDRLEN > filled {
        return None;
    }
    let len = u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap_or_default()) as usize;
    let kind = u16::from_ne_bytes(buf[offset + 4..offset + 6].try_into().unwrap_or_default());
    if len < NLMSG_HDRLEN || offset + len > filled || kind == NLMSG_ERROR || kind == NLMSG_DONE {
        return None;
    }
    Some(len)
}

/// The `(bytes_received, bytes_sent)` pair inside a TCP_INFO attribute.
fn tcp_info_counters(info: &[u8]) -> Option<(u64, u64)> {
    if info.len() < TCP_INFO_MIN_LEN {
        return None;
    }
    Some((
        u64::from_ne_bytes(info[TCPI_BYTES_RECEIVED].try_into().unwrap_or_default()),
        u64::from_ne_bytes(info[TCPI_BYTES_SENT].try_into().unwrap_or_default()),
    ))
}

/// One socket's ports and byte counters from an inet_diag message.
///
/// Walks the rtattr chain that follows the fixed-size header and stops at the
/// first TCP_INFO carrying counters; a message has at most one. A socket
/// without TCP_INFO still counts as an open connection, so it comes back with
/// zeroed counters rather than being dropped. Inode 0 means the kernel did not
/// attach the socket to a file, so it can never be matched back to a process
/// and is dropped here.
pub(super) fn inet_diag_sample(msg: &[u8]) -> Option<SocketSample> {
    if msg.len() < INET_DIAG_MSG_LEN {
        return None;
    }
    let inode = u32::from_ne_bytes(msg[IDIAG_INODE].try_into().unwrap_or_default()) as u64;
    if inode == 0 {
        return None;
    }
    let mut sample = SocketSample {
        inode,
        local_port: u16::from_be_bytes(msg[IDIAG_SPORT].try_into().unwrap_or_default()),
        remote_port: u16::from_be_bytes(msg[IDIAG_DPORT].try_into().unwrap_or_default()),
        remote_is_loopback: is_loopback(msg[IDIAG_FAMILY], &msg[IDIAG_DST]),
        ..SocketSample::default()
    };

    let mut offset = INET_DIAG_MSG_LEN;
    while offset + RTATTR_HDRLEN <= msg.len() {
        let rta_len =
            u16::from_ne_bytes(msg[offset..offset + 2].try_into().unwrap_or_default()) as usize;
        let rta_type =
            u16::from_ne_bytes(msg[offset + 2..offset + 4].try_into().unwrap_or_default());
        if rta_len < RTATTR_HDRLEN || offset + rta_len > msg.len() {
            break;
        }
        if rta_type == INET_DIAG_INFO {
            let payload = &msg[offset + RTATTR_HDRLEN..offset + rta_len];
            if let Some((rx, tx)) = tcp_info_counters(payload) {
                sample.rx_bytes = rx;
                sample.tx_bytes = tx;
                break;
            }
        }
        offset += nl_align(rta_len);
    }
    Some(sample)
}

unsafe fn read_inet_diag_reply(fd: libc::c_int, out: &mut HashMap<u64, SocketSample>) {
    let mut buf = [0u8; NL_RECV_BUF];
    loop {
        let n = libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0);
        if n <= 0 {
            return;
        }
        let filled = n as usize;

        let mut offset = 0;
        while let Some(nl_len) = nlmsg_payload_len(&buf, offset, filled) {
            let msg = &buf[offset + NLMSG_HDRLEN..offset + nl_len];
            if let Some(sample) = inet_diag_sample(msg) {
                out.insert(sample.inode, sample);
            }
            offset += nl_align(nl_len);
        }

        // Running out of buffer mid-header means more datagrams may follow, so
        // recv again. Stopping anywhere else was DONE, ERROR or a malformed
        // length, and each of those ends the reply.
        if offset + NLMSG_HDRLEN <= filled {
            return;
        }
    }
}

fn query_inet_diag(family: u8, out: &mut HashMap<u64, SocketSample>) {
    unsafe {
        let Some(fd) = open_inet_diag(family) else {
            return;
        };
        read_inet_diag_reply(fd, out);
        libc::close(fd);
    }
}

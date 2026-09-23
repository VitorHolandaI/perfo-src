# `src/data/net` — network collection

Produces the `NetSnapshot` that the NET pane (`src/tui/cpu/net.rs`) and its
dashboard summary (`src/tui/net_summary.rs`) draw, and that ships to the
Quickshell widget as JSON because every type here is `Serialize`. Nothing in
this folder draws; it only reads `/proc` and the kernel's sock_diag netlink
interface and turns cumulative counters into rates.

## Files

- **`mod.rs`** — the public surface: `NetInfo`, `NetTotals`, `ProcNet`,
  `ListeningPort`, `PortTraffic`/`PortSide` and the `NetSnapshot` that holds
  them, plus the unit tests for the whole folder. Re-exports `NetMonitor`, the
  only thing callers outside `data::net` touch.
- **`monitor.rs`** — `NetMonitor`, the one stateful piece. Owns the previous
  sample of every counter, so it is what turns deltas into rates, and owns the
  `PortTrafficTracker`. `refresh_with_details` is the entry point: it reads
  `/proc/net/dev`, calls `parse`, takes one `/proc/<pid>/fd` walk and one
  netlink dump, and lends both to `sockets` and `ports`.
- **`parse.rs`** — pure text parsing of `/proc/net/dev` and `/proc/net/snmp`
  into counters. No I/O, no state; called only by `monitor.rs`.
- **`inet_diag.rs`** — the sock_diag netlink ABI, isolated because it is raw
  byte offsets into kernel structs. `tcp_socket_samples()` does one dump per
  address family and returns inode → `SocketSample` (ports, loopback flag,
  byte counters). Called by `monitor.rs`; its `SocketSample` is consumed by
  `sockets.rs` and `ports.rs`.
- **`sockets.rs`** — the `/proc/net/{tcp,udp}` side: socket classes, the
  inode → pid map from `/proc/<pid>/fd`, per-process rollup (`proc_sockets`)
  and the listening-port table (`listening_ports`). Takes the netlink samples
  as an argument rather than fetching them, so the dump happens once.
- **`ports.rs`** — per-port byte accounting. The kernel's counters die with
  each socket, so `PortTrafficTracker` accumulates every socket's delta into
  the port it is charged to and keeps the total after the socket closes.
  `charged_port` holds the attribution rule; `monitor.rs` owns the tracker.

## Flow

`NetMonitor::refresh_with_details`
→ `/proc/net/dev` → `parse::netdev_from` → per-interface rates
→ `sockets::socket_inode_owners()` (one `/proc/<pid>/fd` walk, the expensive step)
→ `inet_diag::tcp_socket_samples()` (one netlink dump)
→ `sockets::proc_sockets(…, &samples)` → `ProcNet` rows
→ `sockets::listening_ports(&owners)` → `ListeningPort` rows, which also give
  the set of served ports
→ `ports::PortTrafficTracker::observe(&samples, &served, elapsed)` → `PortTraffic` rows
→ `NetSnapshot`

## Why the port table is TCP only

The byte counters come from `tcp_info`, which exists only for TCP sockets;
`UDP_DIAG` reports queue depth, not bytes moved. QUIC, DNS and WireGuard
therefore never appear in `PortTraffic`, and the pane says so on its header.

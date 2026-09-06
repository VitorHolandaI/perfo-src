# Process Metrics

## Display identity

The process command line is not a safe display name. It can contain paths,
arguments, JSON fragments, quotes, and arbitrary user-controlled text. Taking
the last `/` from the complete command line is wrong: a QEMU argument such as
`/dev/urandom` changes the displayed name to `urandom"}`; a Firefox child can
become `browser` because its last argument path ends there.

`perfo` emits both:

- `name`: the short kernel process name from the process object.
- `cmd`: the full command line for the detailed TUI.

The Omarchy panel uses `name`, with a first-argument fallback for old
snapshots. The result is a readable label such as `firefox`, `chrome`,
`chromium`, `qemu-system-x86_64`, or `opencode`.

## CPU meaning

The `cpu_percent` field is the process CPU percentage measured by sysinfo
between refreshes. It is not the lifetime average printed by `ps %cpu`.
Thread entries may appear separately, and a threaded process can exceed 100%
when its rows are combined in a detailed view.

The process list is sorted descending by `cpu_percent`. The panel labels the
summary `TOP PROCESSES` so it is not confused with a generic top-level panel.

## Storage attribution

`read_bps` and `write_bps` use `/proc/<pid>/io` `read_bytes` and `write_bytes`.
Those counters represent bytes that reached the storage layer, rather than
all cached syscall traffic. They can be unavailable for another user's
process or a process that exits during sampling; unavailable data degrades to
zero in the current process row and is not an error in the entire snapshot.

## Live update behavior

The `stream --json` command emits one complete JSON object per sample and
flushes stdout after every line. The plugin parses each line and replaces the
snapshot object, which is required for QML bindings to notice changes.

The first counter sample seeds deltas. A consumer must not present that seed
as a fabricated spike or a fabricated zero.

## Comparison with references

- `System stats` gets the short label right with `/proc/<pid>/comm`, but uses
  lifetime `ps %cpu` values.
- `MenuVitals` uses two `/proc` sweeps and computes a live delta only while
  process sections are expanded.
- `System Metrics` uses the same live-delta principle and explicitly explains
  why process lists should be gated.

`perfo` keeps process collection in the Rust engine because its TUI and JSON
consumer already need the same process state.

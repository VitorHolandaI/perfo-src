# `src/` — the perfo crate

Everything perfo does lives here. The binary is a thin shell: `main.rs` only
calls `perfo::run()`, so every code path is reachable from the library and the
integration tests in `../tests/` exercise the same functions the CLI does.

One repository produces two front ends from this code. The TUI under `tui/` is
what `perfo` opens in a terminal; `stream.rs` is what the Omarchy widget drives
over a pipe. They share the collectors in `data/` and the recording format in
`recordings/`, which is why a sample written by one replays in the other.

## Files

- `main.rs` — the binary entry point, two lines. It exists so the crate can be
  both a library and an executable; put nothing here.
- `lib.rs` — argument parsing (`Command`, `parse`) and command dispatch
  (`run`). Each subcommand gets a `run_*` function that owns its exit code.
  `run` is the only place that decides what a given argv means, so a new
  subcommand is a variant in `Command`, an arm in `parse`, and an arm in `run`.
- `stream.rs` — the `stream --json` collector, split out of `lib.rs` because it
  is the one command that outlives a single answer and so is the only one with
  a process lifetime to manage. Holds the control protocol a client speaks over
  stdin (`profile`, `recording`, `mask`, `threads`), and the rules for ending:
  stdin closing when a client was holding it, and a write failing because
  nobody reads the other end. `tests/stream_lifetime.rs` drives the real binary
  for those, because the bug they cover was in how the process ends.
- `bounded_child.rs` — waiting for a child process with a budget, then killing
  and reaping it. Shared because two unrelated places spawn a short-lived
  helper that must not hold the caller: the `nvidia-smi` sampler in
  `data/nvidia_smi/`, where a wedged driver kept the collector from its first
  snapshot, and `tui/clipboard.rs`, which runs on a keypress. Both used a bare
  `wait()`, which has no deadline at all.
- `theme.rs` — reads the Omarchy theme so the TUI matches the desktop. Returns
  `None` when there is no theme to read, and every caller falls back to its own
  default rather than failing.
- `units.rs` — conversions between the raw counters the collectors read and the
  percentages and widths the UI renders, plus the shared constants. Everything
  here is pure, so it is the cheapest place to put a rule that both front ends
  must agree on.

## Folders

- `data/` — the collectors. Reads `/proc`, `/sys` and hwmon, and decides what
  to gather from a `CollectionPlan`. See its README; the gating model there is
  the thing most likely to surprise you.
- `recordings/` — the on-disk recording format, its reduction to a timeline and
  its offset index. See its README before changing what a sample holds.
- `tui/` — the terminal interface: panes, key handling and the history
  recorder. See `tui/history/README.md` for the recording side.
- `trace/` — the `perfo trace` syscall tracer, attached by ptrace.

## Flow

```
main.rs → lib.rs::run → parse(argv)
                      ├─ stream.rs::run_stream ──┐
                      ├─ tui::run ───────────────┤→ data/ (collectors)
                      ├─ recordings::dispatch ───┘
                      └─ trace/
```

`data/` is the floor: both front ends get their numbers from it and neither
reads a sensor directly.

## Working here

```bash
cargo test                        # the whole suite, never filtered
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

Files stay under 500 lines and functions under 80. When a file crosses the
line, split it by responsibility and give the new file a module doc saying what
it owns, the way `stream.rs` does. Update this README in the same commit.

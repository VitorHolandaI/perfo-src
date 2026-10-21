# `src/data` — where every number comes from

Everything the TUI and the widget display is collected here, and nothing in
this folder draws. Each module owns one source of truth in `/proc` or `/sys`,
turns it into a plain serialisable struct, and hands it on. The rule that
holds the folder together: a reader never reaches into another reader's
files, so adding a metric means touching one module.

Callers outside this folder see `cpu::CpuMonitor` (the whole machine) or
`summary::WidgetSummaryMonitor` (the cheap subset), never the individual
readers.

## Files

- **`mod.rs`** — the module list and nothing else.
- **`cpu/`** — the aggregate monitor. `CpuMonitor` owns every other reader in
  this folder and decides, from a `CollectionPlan`, which of them run on a
  given tick. It is what `src/lib.rs` and `src/tui/` talk to.
- **`summary.rs`** — `WidgetSummaryMonitor`, a second, far smaller monitor for
  the bar widget's always-on collector: CPU, memory, GPU, NPU and per
  interface packet loss. It exists because the widget runs a collector for
  the whole session and cannot pay for the full snapshot. Anything added here
  is paid for continuously, which is why it reads `net::NetLossMonitor` and
  not `net::NetMonitor`.
- **`net/`** — interfaces, sockets, per-port traffic and packet loss. Has its
  own README.
- **`disk/`** — per-device I/O rates, queue depth, filesystems and disk
  temperature. Also owns `rate()` and `HISTORY_SAMPLES`, which the network
  and CPU readers reuse for their sparkline rings.
- **`mem.rs`** — `/proc/meminfo` into used, available, swap, and the pressure
  figures it gets from `psi.rs`.
- **`psi.rs`** — `/proc/pressure/*`, the kernel's stall metrics. Read by
  `mem.rs` and by `disk/`, which is why it is not inside either.
- **`fan.rs`** — fan RPM from the hwmon tree.
- **`gpu/`** — AMD, Intel and NVIDIA cards behind one `GpuMonitor`. AMD and
  Intel read DRM sysfs directly; NVIDIA needs a vendor library and lives in
  the two files below.
- **`nvidia.rs`** — NVML through `dlopen`: the full set, including
  per-process GPU utilisation. Only reachable from a dynamically linked
  build, so in the published binary it is the path that is never taken.
- **`nvidia_smi.rs`** — the same metrics from the `nvidia-smi` that ships with
  every driver, as a child process that the refresh loop never waits on.
  This is what the released musl build actually uses, because a statically
  linked musl program has no dynamic loader and its `dlopen` always returns
  NULL. `gpu/monitor.rs` tries `nvidia.rs` first and falls through to here.
- **`npu.rs`** — Intel NPU utilisation from the accel sysfs tree.

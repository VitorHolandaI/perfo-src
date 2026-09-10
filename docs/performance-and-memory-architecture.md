# Performance and Memory Architecture

This document details the performance, memory, and lifecycle design decisions across `perfo` (Rust backend) and the Omarchy desktop plugin (QML/Quickshell frontend).

---

## 1. Executive Summary and System Benchmarks

All measurements below were benchmarked on x86_64 Linux (Intel Core Ultra 7 with Arc Graphics and Intel NPU) using optimized release binaries (`--release --locked`).

### Collector and Refresh Latency

| Operational Profile | Scope / Subsystems Polled | Latency / Tick | Relative Speedup |
| :--- | :--- | :--- | :--- |
| **Widget Summary Monitor** (`stream --json --summary`) | CPU aggregate, memory, GPU summary, NPU summary | **0.01 ms** | **11,927x faster** than Full |
| **TUI Dashboard** (Default non-fullscreen) | CPU overall, memory, disk rates, net rates, GPU, NPU | **29.30 ms** | **2.8x faster** than Full |
| **TUI CPU Fullscreen** (`1:CPU`) | CPU per-core, temps, process list, CPU/RAM/affinity | **22.76 ms** | **3.6x faster** than Full |
| **Full / History Collector** (`perfo stream --json`, `HIST`) | All cores, temps, all processes, I/O rates, net sockets, GPU engines | **81.79 ms** | Baseline |

### Memory Footprint and Stream Payloads

| Component / Scenario | Raw Payload / Footprint | Optimized Architecture | Net Gain |
| :--- | :--- | :--- | :--- |
| **Closed Bar Widget Stream** | ~600,000 bytes / sample (JSON) | **182 bytes / sample** (JSON) | **3,300x reduction** in I/O |
| **Quickshell Heap (Idle Bar)** | ~3.5% CPU (JSON.parse storms) | **0.2% CPU** | Near-zero idle overhead |
| **2h Flight Recorder Replay** | ~2,000 MB heap (raw JSON.parse) | **~38 MB net RSS** | **52x lower memory** |
| **TUI Process RSS** | Monolithic process table | **10.5 MB (Dashboard)** / **12.3 MB (CPU)** | Under 13 MB resident |
| **TUI First Render Latency** | N/A | **1.5 ms to first frame** | Instant terminal startup |

---

## 2. Adaptive Widget Stream Architecture

### The Problem

The desktop bar widget is permanently visible on the user's screen. When the widget panel is closed, the user only requires compact headline metrics:
- CPU overall percent
- Memory used and total percent
- GPU aggregate utilization
- NPU aggregate utilization

Previously, `perfo stream --json` unconditionally serialized complete per-process snapshots, socket inodes, I/O windows, and fan speeds every second (~600 KB/s). Deserializing 600 KB JSON strings every 1,000 ms inside QML/QJSEngine caused:
- Continuous 3% to 5% CPU usage by the desktop shell.
- Premature garbage collector churn and micro-stutters during desktop animations.

### The Solution

1. **Dedicated Summary Monitor (`WidgetSummaryMonitor`)**:
   - Implemented in `src/data/summary.rs`.
   - Initializes sysinfo with strictly `CpuRefreshKind::everything()` and `MemoryRefreshKind::everything()`.
   - Skips all `/proc/<pid>/` process scans, socket tables, disk partitions, and thermal hardware zones.
   - Emits a minimal JSON schema averaging 182 bytes:
     ```json
     {"overall_percent":4.8,"total_mem_bytes":16164474880,"used_mem_bytes":7113834496,"gpu":{"devices":[{"usage_percent":2.7}]},"npu":{"devices":[{"utilization_percent":0.0}]}}
     ```
2. **Adaptive Process Switching (`BarWidget.qml`)**:
   - `BarWidget.qml` controls two discrete background processes:
     - `summaryCollector`: runs `perfo stream --json --summary` when `!needsDetails`.
     - `detailCollector`: runs `perfo stream --json` when `needsDetails` (`opened || isRecording`).
   - When switching states, late stdout lines from the terminated collector are ignored via state guards.
3. **GPU Rate Throttling**:
   - GPU summary polling runs on a 2-second cadence (`GPU_SAMPLE_INTERVAL`) to prevent Intel DRM fdinfo lock contention while the bar is idle.

---

## 3. TUI Selective Collection Plan and Needs Union

### The Problem

1. **Dashboard Overcollection**: The default TUI view displays an aggregate 6-card summary. Running a full collection loop incurred ~82 ms latency reading per-process I/O and network sockets that were never displayed.
2. **Recording Override Starvation Bug**: Previously, activating session recording forced `collection_profile(&state)` to return `CollectionProfile::History`. When the user navigated to the CPU, IO, Net, or Memory views while recording, `History` mode omitted `cpu_details`, `cpu_temperatures`, `disk_details`, and `process_affinity`.
3. Furthermore, process list preparation (`prepare(s, &mut state)`) was guarded by `active_profile == CollectionProfile::Cpu`. Because the profile was forced to `History`, `prepare()` produced an empty process table, breaking core filtering, process navigation, and process killing during recordings.
4. **Misleading Device Detection**: On tick 0, GPU and NPU delta calculations are not yet available. Metric cards displayed `"not detected"` even when hardware was physically present and operational.

### The Solution

1. **`CollectionPlan` and `CollectionNeeds` Union**:
   - Defined in `src/data/cpu.rs`:
     ```rust
     #[derive(Clone, Copy, Debug, PartialEq, Eq)]
     pub struct CollectionPlan {
         pub visible: CollectionProfile,
         pub recording: bool,
     }

     impl CollectionPlan {
         pub fn needs(self) -> CollectionNeeds {
             let visible_needs = self.visible.needs();
             if self.recording {
                 visible_needs.union(CollectionProfile::History.needs())
             } else {
                 visible_needs
             }
         }
     }
     ```
   - `CollectionNeeds::union()` performs a bitwise OR across all 22 subsystem dependencies (CPU details, temperatures, process I/O, network listeners, GPU engines, etc.).
   - Both visible requirements and flight recorder requirements are polled together without starvation.
2. **Generic `CpuMonitor` API**:
   - `new_for`, `refresh_for`, and `snapshot_for` take `impl Into<CollectionPlan>`.
   - `CollectionProfile` implements `From<CollectionProfile> for CollectionPlan`, preserving backward compatibility with zero breaking changes for existing CLI tests.
3. **Loop Logic Separation**:
   - Process preparation condition: `if active_plan.visible == CollectionProfile::Cpu`.
   - History recording condition: `if active_plan.recording || active_plan.visible == CollectionProfile::History`.
4. **Hardware Sampling State Warm-Up**:
   - Helper `metric_card_display(value: Option<f32>, detected: bool) -> String`:
     - Value present: `format!("{:>3.0}%  {}", percent, bar(percent, 12))`
     - Value `None` but `detected == true`: `"--% (sampling)"`
     - Value `None` and `detected == false`: `"not detected"`

---

## 4. On-Demand Process Streaming and Binary Offset Indexing for 2h+ Recordings

### The Problem

A 2-hour flight recorder recording at 1 Hz contains 7,200 full snapshots, producing a ~119 MB raw JSON file on disk.

When loaded into QML via `JSON.parse(fileContent)`:
- QJSEngine allocates individual JavaScript objects for every process row across all 7,200 seconds.
- Heap usage surged to **1.5 GB to 2.1 GB**, triggering multi-second GC pauses that completely froze the desktop UI.

### The Solution

A two-tier streaming model was implemented to guarantee 100% data preservation on disk while capping frontend RAM under 40 MB:

```
Recorded Session Disk Structure:
├── session_20260908_120000.json          (119 MB: Full raw snapshots, unchanged)
├── session_20260908_120000.timeline.json (1.3 MB: Scalar metrics + top proc name)
└── session_20260908_120000.offsets.bin   (113 KB: Binary seek table of (u64, u64))
```

1. **Companion Timeline Cache (`.timeline.json`)**:
   - Generated on first load via `perfo record timeline <target>`.
   - Extracts scalar metrics (`timestamp`, `cpu`, `mem`, `gpu`, `read_bps`, `write_bps`, `net_rx_bps`, `net_tx_bps`, `top_process_name`).
   - Strips the thousands of process entries per second, reducing 119 MB down to ~1.3 MB.
   - Subsequent loads complete in ~1 ms.
2. **Binary Offset Table (`.offsets.bin`)**:
   - Stores fixed 16-byte pairs: `(u64 start_byte, u64 end_byte)` marking the exact file slice of each sample's `{...}` block.
3. **Direct Byte Seeking (`perfo record inspect`)**:
   - Syntax: `perfo record inspect <target> <index> [window]`.
   - Seeks directly to the byte offset in the 119 MB file using kernel `lseek(2)`.
   - Reads only the requested slice (e.g. 25 KB to 50 KB), wraps it in `[...]`, and outputs only those process lists.
   - Query latency dropped from **~750 ms to ~13 ms**.
4. **Fast Header Inspection in Session List**:
   - Listing recorded sessions (`perfo record list`) reads only the first 4,096 bytes of each file to extract timestamps and durations without reading multi-megabyte sample arrays (~3 ms total for all files).
5. **QML Sliding Window and Debounced Prefetching**:
   - `loadedHistory` receives only the 1.3 MB timeline array.
   - Full process rows are cached in a sliding window ($\pm 25$ seconds).
   - During playback (1x, 2x): seamless playback from cache with automatic background prefetch when within 5s of the window boundary.
   - During scrubbing or rapid timeline jumping (`[` and `]`): timeline needle moves immediately at 60 FPS displaying the top process name; backend inspect calls are debounced by 80 ms.

---

## 5. In-Place QML Array Reactivity Tracking

### The Problem

In `HistoryPage.qml` and `Panel.qml`, live monitoring appends new samples in real time to the session buffer.
Using JavaScript array push (`activeHistory.push(newSample)`) modified the array contents in place without changing the object reference. Because QML property bindings listen to reference changes, derived expressions like:
```qml
property int durationSeconds: activeHistory.length > 0 ? (activeHistory[activeHistory.length - 1].timestamp - activeHistory[0].timestamp) : 0
```
failed to re-evaluate. The timeline ruler and elapsed time labels remained stuck (e.g. `00:00 / 00:00`) unless the user explicitly clicked the LIVE button to force a property reassignment.

### The Solution

- Introduced an explicit reactive synchronization property:
  ```qml
  property int activeHistoryLength: 0
  ```
- Every time a sample is pushed or unshifted, `activeHistoryLength` is incremented.
- Bound all timeline rulers, duration labels, and needle tracking to `activeHistoryLength`:
  ```qml
  readonly property int activeHistoryCount: activeHistoryLength
  ```
- Result: 60 FPS reactive updates during live monitoring without cloning multi-megabyte arrays on every tick.

---

## 6. Native Intel NPU Hardware Integration

### The Problem

Intel Core Ultra processors feature an integrated Neural Processing Unit (NPU) exposed under Linux as an accelerator device. Tools like `intel-npu-smi` require separate installation and root privileges, making them unsuitable for an unprivileged desktop widget.

### The Solution

- Direct sysfs discovery under `/sys/class/accel/accel*`:
  - Validates `device/vendor == "0x8086"`.
  - Validates `device/driver == "intel_vpu"`.
- Utilization counters are read directly from kernel sysfs metrics without external daemons.
- Counter resets are detected safely and clamped against wrap-around spikes.
- Graceful degradation: if no NPU is detected or the kernel module is absent, the subsystem emits an empty device list without errors or log spam.

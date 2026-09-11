//! The single refresh pass that fills a snapshot.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use sysinfo::{
    Components, CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System, UpdateKind,
};

use crate::data::disk::DiskMonitor;
use crate::data::fan::{FanMonitor, FanSnapshot};
use crate::data::gpu::{GpuMonitor, GpuSnapshot};
use crate::data::mem;
use crate::data::mem::MemSnapshot;
use crate::data::net::{NetMonitor, NetSnapshot};
use crate::data::npu::{NpuMonitor, NpuSnapshot};

use super::collection::{CollectionNeeds, CollectionPlan};
use super::process::*;
use super::topology::*;
use super::CpuSnapshot;

/// Samples CPU + process data via sysinfo.
///
/// CPU usage is a delta over the time between two refreshes, so callers must
/// space `refresh()` calls about 1s apart (see `wait_sample_interval`). The
/// first `refresh()` after construction seeds the deltas and should be ignored.
pub struct CpuMonitor {
    sys: System,
    components: Components,
    disks: DiskMonitor,
    fans: FanMonitor,
    gpu: GpuMonitor,
    npu: NpuMonitor,
    net: NetMonitor,
    users_cache: HashMap<u32, String>,
    /// pid -> last-run CPU, refreshed only on full process refreshes.
    last_cpu: HashMap<u32, u32>,
    /// pid -> (read_bytes, write_bytes) from the previous full refresh.
    io_prev: HashMap<u32, (u64, u64)>,
    /// pid -> bytes/second, computed on full refreshes.
    io_rates: HashMap<u32, (u64, u64)>,
    /// pid -> bytes accumulated since the window started (see IO_WINDOW_SECS).
    io_window: HashMap<u32, (u64, u64)>,
    /// When the current I/O window started.
    io_window_start: Instant,
    /// (iowait, total) counters from the previous /proc/stat read.
    stat_prev: (u64, u64),
    /// When the last full refresh happened (drives I/O rate conversion).
    last_full: Option<Instant>,
    /// I/O wait percentage between the last two /proc/stat samples.
    iowait_percent: f32,
    /// P/E/L classification, probed once via CPUID 0x1A (pinned threads).
    core_types: Vec<CoreType>,
    /// Per-core maximum frequencies, stable until CPU topology changes.
    max_freq_mhz: Vec<u64>,
    /// Overall CPU usage ring (newest last) for the history graph.
    history: VecDeque<f32>,
}

impl CpuMonitor {
    pub fn new() -> Self {
        Self::new_with_needs(CollectionNeeds::full())
    }

    pub fn new_for(plan: impl Into<CollectionPlan>) -> Self {
        Self::new_with_needs(plan.into().needs())
    }

    fn new_with_needs(needs: CollectionNeeds) -> Self {
        // new_all() does a heavyweight first refresh (disks, net, users,
        // processes with everything); build only CPU + memory here and let
        // do_refresh handle the rest, so startup stays cheap.
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let mut monitor = Self {
            sys,
            components: Components::new_with_refreshed_list(),
            disks: DiskMonitor::new(),
            fans: FanMonitor::new(),
            gpu: GpuMonitor::new(),
            npu: NpuMonitor::new(),
            net: NetMonitor::new(),
            users_cache: HashMap::new(),
            last_cpu: HashMap::new(),
            io_prev: HashMap::new(),
            io_rates: HashMap::new(),
            io_window: HashMap::new(),
            io_window_start: Instant::now(),
            stat_prev: (0, 0),
            last_full: None,
            iowait_percent: 0.0,
            core_types: Vec::new(),
            max_freq_mhz: Vec::new(),
            history: VecDeque::new(),
        };
        monitor.max_freq_mhz = per_core_max_freqs();
        monitor.core_types = core_types_of(&monitor.max_freq_mhz);
        // Seed counter deltas only for providers required by the initial view.
        monitor.refresh_needs(needs, true);
        monitor
    }

    fn refresh_processes(&mut self, needs: CollectionNeeds) {
        let mut refresh = ProcessRefreshKind::nothing()
            .with_user(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_exe(UpdateKind::OnlyIfNotSet);
        if needs.process_cpu {
            refresh = refresh.with_cpu();
        }
        if needs.process_memory {
            refresh = refresh.with_memory();
        }
        if needs.process_tasks {
            refresh = refresh.with_tasks();
        }
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    }

    /// Full refresh: CPU + memory + all process stats + last-run CPU map.
    /// Expensive (tens of thousands of /proc reads); call sparingly (~every 2s).
    pub fn refresh(&mut self) {
        self.refresh_needs(CollectionNeeds::full(), true);
    }

    pub fn refresh_for(&mut self, plan: impl Into<CollectionPlan>, detailed_tick: bool) {
        self.refresh_needs(plan.into().needs(), detailed_tick);
    }

    fn refresh_needs(&mut self, needs: CollectionNeeds, detailed_tick: bool) {
        if needs.cpu {
            self.sys.refresh_cpu_usage();
        }
        if needs.memory {
            self.sys.refresh_memory();
        }
        if detailed_tick && needs.processes {
            self.refresh_processes(needs);
        }
        if needs.cpu_temperatures {
            self.components.refresh(false);
        }
        if detailed_tick && needs.disks {
            if needs.disk_details {
                self.disks.refresh();
            } else {
                self.disks.refresh_summary();
            }
        }
        if needs.gpu && (detailed_tick || needs.gpu_processes) {
            if needs.gpu_processes {
                self.gpu.refresh();
            } else {
                self.gpu.refresh_summary();
            }
        }
        if needs.npu {
            self.npu.refresh();
        }
        if detailed_tick && needs.network {
            self.net
                .refresh_with_details(needs.network_processes, needs.network_listeners);
        }
        if detailed_tick && needs.process_affinity {
            self.refresh_process_affinity();
        }
        if detailed_tick && needs.process_io {
            self.refresh_process_io();
        }
        if needs.io_wait {
            self.refresh_iowait();
        }
    }

    fn refresh_process_affinity(&mut self) {
        self.last_cpu = self
            .sys
            .processes()
            .keys()
            .filter_map(|pid| last_cpu_of(pid.as_u32()).map(|cpu| (pid.as_u32(), cpu)))
            .collect();
    }

    fn refresh_process_io(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .last_full
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let mut io_cur: HashMap<u32, (u64, u64)> =
            HashMap::with_capacity(self.sys.processes().len());
        let mut io_rates: HashMap<u32, (u64, u64)> =
            HashMap::with_capacity(self.sys.processes().len());
        let mut io_deltas: HashMap<u32, (u64, u64)> =
            HashMap::with_capacity(self.sys.processes().len());
        for pid in self.sys.processes().keys() {
            let pid = pid.as_u32();
            // /proc/<pid>/io is one tiny file per process; the page cache
            // keeps the read cheap (~µs) once warm.
            if let Some((rb, wb)) = proc_io_of(pid) {
                if let Some((prb, pwb)) = self.io_prev.get(&pid) {
                    let read_delta = rb.saturating_sub(*prb);
                    let write_delta = wb.saturating_sub(*pwb);
                    io_rates.insert(
                        pid,
                        (
                            rate_bps(read_delta, elapsed),
                            rate_bps(write_delta, elapsed),
                        ),
                    );
                    io_deltas.insert(pid, (read_delta, write_delta));
                }
                io_cur.insert(pid, (rb, wb));
            }
        }
        // Rolling per-process window: reset every IO_WINDOW_SECS so the
        // "who hammered the disk" list is a bounded recent history.
        if now.duration_since(self.io_window_start).as_secs() >= IO_WINDOW_SECS {
            self.io_window.clear();
            self.io_window_start = now;
        }
        for (pid, (rb, wb)) in io_deltas {
            let e = self.io_window.entry(pid).or_insert((0, 0));
            e.0 = e.0.saturating_add(rb);
            e.1 = e.1.saturating_add(wb);
        }
        self.io_prev = io_cur;
        self.io_rates = io_rates;
        self.last_full = Some(now);
    }

    fn refresh_iowait(&mut self) {
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let (iw, tot) = stat_iowait_from(&stat);
        self.iowait_percent = iowait_percent(self.stat_prev, (iw, tot));
        self.stat_prev = (iw, tot);
    }

    pub fn snapshot(&mut self) -> CpuSnapshot {
        self.snapshot_needs(CollectionNeeds::full())
    }

    pub fn snapshot_for(&mut self, plan: impl Into<CollectionPlan>) -> CpuSnapshot {
        self.snapshot_needs(plan.into().needs())
    }

    fn snapshot_needs(&mut self, needs: CollectionNeeds) -> CpuSnapshot {
        // On Linux every thread appears as its own /proc entry; map each
        // thread tid to its owning process via Process::tasks().
        let mut task_of: HashMap<u32, u32> = HashMap::new();
        if needs.processes {
            for (pid, p) in self.sys.processes() {
                if let Some(tids) = p.tasks() {
                    for t in tids {
                        task_of.insert(t.as_u32(), pid.as_u32());
                    }
                }
            }
        }

        let overall_percent = if needs.cpu {
            self.sys.global_cpu_usage()
        } else {
            0.0
        };
        if needs.cpu {
            self.history.push_back(overall_percent);
            if self.history.len() > crate::data::disk::HISTORY_SAMPLES {
                self.history.pop_front();
            }
        }
        // iowait needs a delta between two /proc/stat samples; stat_prev
        // holds the latest, so the first snapshot reports 0.
        let per_core = if needs.cpu_details {
            self.sys.cpus().iter().map(|cpu| cpu.cpu_usage()).collect()
        } else {
            Vec::new()
        };
        let per_core_types = if needs.cpu_details {
            self.core_types.clone()
        } else {
            Vec::new()
        };
        let per_core_freq_mhz = if needs.cpu_details {
            self.sys.cpus().iter().map(|cpu| cpu.frequency()).collect()
        } else {
            Vec::new()
        };
        let per_core_max_freq_mhz = if needs.cpu_details {
            self.max_freq_mhz.clone()
        } else {
            Vec::new()
        };
        let cpu_temp_c = needs
            .cpu_temperatures
            .then(|| cpu_temperature(&self.components))
            .flatten();
        let per_core_temp_c = if needs.cpu_temperatures {
            per_core_temps(&self.components)
        } else {
            Vec::new()
        };
        let load_avg = if needs.cpu {
            let load = sysinfo::System::load_average();
            [load.one, load.five, load.fifteen]
        } else {
            [0.0; 3]
        };

        // Username lookups hit NSS; cache them per uid instead of resolving
        // every process every tick.
        let mut cache = std::mem::take(&mut self.users_cache);
        let mut processes: Vec<ProcessInfo> = if needs.processes {
            self.sys
                .processes()
                .iter()
                .map(|(pid, p)| {
                    let pid_u = pid.as_u32();
                    let owner = task_of.get(&pid_u).copied();
                    let user = match p.user_id().map(|u| **u) {
                        Some(uid) => match cache.get(&uid) {
                            Some(n) => n.clone(),
                            None => {
                                let n = user_name_of(uid);
                                cache.insert(uid, n.clone());
                                n
                            }
                        },
                        None => "?".into(),
                    };
                    let cmd = if p.cmd().is_empty() {
                        p.name().to_string_lossy().into_owned()
                    } else {
                        p.cmd()
                            .iter()
                            .map(|s| s.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    let (read_bps, write_bps) =
                        self.io_rates.get(&pid_u).copied().unwrap_or((0, 0));
                    let (win_read_bytes, win_write_bytes) =
                        self.io_window.get(&pid_u).copied().unwrap_or((0, 0));
                    ProcessInfo {
                        pid: pid_u,
                        name: p.name().to_string_lossy().into_owned(),
                        ppid: p.parent().map(|pp| pp.as_u32()),
                        owner,
                        is_kernel: pid_u == 2 || p.parent().map(|pp| pp.as_u32()) == Some(2),
                        user,
                        cpu_percent: p.cpu_usage(),
                        mem_bytes: p.memory(),
                        cmd,
                        last_cpu: self.last_cpu.get(&pid_u).copied(),
                        read_bps,
                        write_bps,
                        win_read_bytes,
                        win_write_bytes,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        if needs.processes {
            let active_uids: HashSet<u32> = self
                .sys
                .processes()
                .values()
                .filter_map(|process| process.user_id().map(|uid| **uid))
                .collect();
            cache.retain(|uid, _| active_uids.contains(uid));
        }
        self.users_cache = cache;
        processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));

        let mem = if needs.memory_details {
            mem::snapshot()
        } else if needs.memory {
            MemSnapshot {
                total: self.sys.total_memory(),
                used: self.sys.used_memory(),
                ..MemSnapshot::default()
            }
        } else {
            MemSnapshot::default()
        };

        CpuSnapshot {
            fans: if needs.fans {
                self.fans.snapshot()
            } else {
                FanSnapshot::default()
            },
            gpu: if needs.gpu {
                self.gpu.snapshot()
            } else {
                GpuSnapshot::default()
            },
            npu: if needs.npu {
                self.npu.snapshot()
            } else {
                NpuSnapshot::default()
            },
            overall_percent,
            iowait_percent: if needs.io_wait {
                self.iowait_percent
            } else {
                0.0
            },
            per_core,
            core_count: if needs.cpu_details {
                self.sys.cpus().len()
            } else {
                0
            },
            per_core_types,
            per_core_freq_mhz,
            per_core_max_freq_mhz,
            per_core_temp_c,
            cpu_temp_c,
            load_avg,
            total_mem_bytes: if needs.memory {
                self.sys.total_memory()
            } else {
                0
            },
            used_mem_bytes: if needs.memory {
                self.sys.used_memory()
            } else {
                0
            },
            mem,
            io_pressure_some: if needs.disks {
                self.disks.io_pressure()
            } else {
                [0.0; 3]
            },
            io_history: if needs.disks {
                self.disks.history().clone()
            } else {
                HashMap::new()
            },
            disks: if needs.disks {
                self.disks
                    .snapshot_with_temperatures(needs.disk_temperatures)
            } else {
                Vec::new()
            },
            net: if needs.network {
                self.net.snapshot()
            } else {
                NetSnapshot::default()
            },
            cpu_history: if needs.cpu {
                self.history.clone()
            } else {
                VecDeque::new()
            },
            processes,
        }
    }
}

impl Default for CpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Bytes per second from a refresh-interval delta.
pub(crate) fn rate_bps(delta: u64, elapsed_secs: f32) -> u64 {
    if elapsed_secs <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed_secs as f64) as u64
    }
}

/// Sleep long enough for a fresh CPU/process delta to accumulate.
pub fn wait_sample_interval() {
    std::thread::sleep(Duration::from_millis(1000));
}

/// Length of the per-process I/O accumulation window (seconds).
const IO_WINDOW_SECS: u64 = 300;

//! Turning the collected readings into a snapshot.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use crate::data::fan::FanSnapshot;
use crate::data::gpu::GpuSnapshot;
use crate::data::mem;
use crate::data::mem::MemSnapshot;
use crate::data::net::NetSnapshot;
use crate::data::npu::NpuSnapshot;

use super::collection::{CollectionNeeds, CollectionPlan};
use super::process::{user_name_of, ProcessInfo};
use super::topology::{cpu_temperature, per_core_temps, CoreType};
use super::CpuSnapshot;

/// The CPU half of a snapshot, gathered in one place so snapshot_needs reads
/// as the assembly it is.
struct CpuFields {
    overall_percent: f32,
    per_core: Vec<f32>,
    per_core_types: Vec<CoreType>,
    per_core_freq_mhz: Vec<u64>,
    per_core_max_freq_mhz: Vec<u64>,
    cpu_temp_c: Option<f32>,
    per_core_temp_c: Vec<Option<f32>>,
    load_avg: [f64; 3],
}

use super::CpuMonitor;

impl CpuMonitor {
    pub fn snapshot(&mut self) -> CpuSnapshot {
        self.snapshot_needs(CollectionNeeds::full())
    }

    pub fn snapshot_for(&mut self, plan: impl Into<CollectionPlan>) -> CpuSnapshot {
        self.snapshot_needs(plan.into().needs())
    }

    /// Every thread shows up as its own /proc entry; map each tid back to the
    /// process that owns it.
    fn thread_owner_map(&self, needs: CollectionNeeds) -> HashMap<u32, u32> {
        let mut task_of: HashMap<u32, u32> = HashMap::new();
        if !needs.processes {
            return task_of;
        }
        for (pid, p) in self.sys.processes() {
            if let Some(tids) = p.tasks() {
                for t in tids {
                    task_of.insert(t.as_u32(), pid.as_u32());
                }
            }
        }
        task_of
    }

    /// The per-core and whole-CPU numbers, each gated on whether the caller
    /// asked for that level of detail.
    fn cpu_fields(&mut self, needs: CollectionNeeds) -> CpuFields {
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
        CpuFields {
            overall_percent,
            per_core,
            per_core_types,
            per_core_freq_mhz,
            per_core_max_freq_mhz,
            cpu_temp_c,
            per_core_temp_c,
            load_avg,
        }
    }

    /// The process table for this snapshot, with usernames resolved through a
    /// per-uid cache because NSS lookups are expensive at this rate.
    fn process_list(
        &mut self,
        needs: CollectionNeeds,
        task_of: &HashMap<u32, u32>,
    ) -> Vec<ProcessInfo> {
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
        processes
    }

    fn snapshot_needs(&mut self, needs: CollectionNeeds) -> CpuSnapshot {
        let task_of = self.thread_owner_map(needs);
        let CpuFields {
            overall_percent,
            per_core,
            per_core_types,
            per_core_freq_mhz,
            per_core_max_freq_mhz,
            cpu_temp_c,
            per_core_temp_c,
            load_avg,
        } = self.cpu_fields(needs);

        let processes = self.process_list(needs, &task_of);

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
pub(super) const IO_WINDOW_SECS: u64 = 300;

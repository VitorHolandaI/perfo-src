//! Building the monitor and pulling one round of fresh readings.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use sysinfo::{
    Components, CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System, UpdateKind,
};

use crate::data::disk::DiskMonitor;
use crate::data::fan::FanMonitor;
use crate::data::gpu::GpuMonitor;
use crate::data::net::NetMonitor;
use crate::data::npu::NpuMonitor;

use super::collection::{CollectionNeeds, CollectionPlan};
use super::process::{iowait_percent, last_cpu_of, proc_io_of, stat_iowait_from};
use super::topology::{core_types_of, per_core_max_freqs};

use super::monitor::{rate_bps, IO_WINDOW_SECS};
use super::CpuMonitor;

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
}

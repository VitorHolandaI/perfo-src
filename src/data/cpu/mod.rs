//! CPU, memory and process collection.

mod collection;
mod monitor;
mod process;
mod refresh;
mod topology;

pub use collection::{CollectionNeeds, CollectionPlan, CollectionProfile, RecordingMask};
pub use monitor::wait_sample_interval;
pub use process::ProcessInfo;
pub use topology::CoreType;

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use std::time::Instant;

use sysinfo::{Components, System};

use crate::data::disk::{DiskInfo, DiskMonitor};
use crate::data::fan::FanMonitor;
use crate::data::gpu::GpuMonitor;
use crate::data::net::NetMonitor;
use crate::data::npu::NpuMonitor;

use crate::data::fan::FanSnapshot;
use crate::data::gpu::GpuSnapshot;
use crate::data::mem::MemSnapshot;
use crate::data::net::NetSnapshot;
use crate::data::npu::NpuSnapshot;

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

#[derive(Serialize)]
pub struct CpuSnapshot {
    pub fans: FanSnapshot,
    pub gpu: GpuSnapshot,
    pub npu: NpuSnapshot,
    pub overall_percent: f32,
    /// % of time CPUs spent waiting on disk I/O (from /proc/stat iowait).
    pub iowait_percent: f32,
    pub per_core: Vec<f32>,
    pub core_count: usize,
    pub per_core_types: Vec<CoreType>,
    pub per_core_freq_mhz: Vec<u64>,
    pub per_core_max_freq_mhz: Vec<u64>,
    pub per_core_temp_c: Vec<Option<f32>>,
    pub cpu_temp_c: Option<f32>,
    pub load_avg: [f64; 3],
    pub total_mem_bytes: u64,
    pub used_mem_bytes: u64,
    pub mem: MemSnapshot,
    /// PSI I/O pressure "some" (10s/60s/300s) from /proc/pressure/io.
    pub io_pressure_some: [f64; 3],
    pub disks: Vec<DiskInfo>,
    /// Per-device read/write rate rings (newest last) for IO sparklines.
    pub io_history: HashMap<String, (VecDeque<f32>, VecDeque<f32>)>,
    /// Per-interface network rates + TCP totals.
    pub net: NetSnapshot,
    /// Overall CPU usage ring (newest last) for the CPU history graph.
    pub cpu_history: VecDeque<f32>,
    pub processes: Vec<ProcessInfo>,
}

#[cfg(test)]
mod tests {
    use super::collection::*;
    use super::monitor::rate_bps;
    use super::process::*;
    use super::topology::*;

    #[test]
    fn dashboard_collects_only_aggregate_providers() {
        let needs = CollectionProfile::Dashboard.needs();
        assert!(needs.cpu);
        assert!(needs.cpu_details);
        assert!(needs.cpu_temperatures);
        assert!(needs.memory);
        assert!(needs.disks);
        assert!(needs.disk_details);
        assert!(needs.network);
        assert!(needs.gpu);
        assert!(needs.npu);
        assert!(needs.processes);
        assert!(!needs.process_io);
        assert!(!needs.process_affinity);
        assert!(!needs.disk_temperatures);
        // The dashboard strip shows cooler RPM.
        assert!(needs.fans);
    }

    #[test]
    fn fans_profile_reads_only_coolers_and_temperature() {
        let needs = CollectionProfile::Fans.needs();
        assert!(needs.fans);
        assert!(needs.cpu_temperatures);
        assert!(!needs.processes);
        assert!(!needs.disks);
        assert!(!needs.network);
        assert!(!needs.gpu);
    }

    #[test]
    fn detail_profiles_enable_only_their_dependencies() {
        assert_eq!(
            CollectionProfile::Cpu.needs(),
            CollectionNeeds {
                cpu: true,
                cpu_details: true,
                cpu_temperatures: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                process_tasks: true,
                process_affinity: true,
                ..CollectionNeeds::default()
            }
        );
        assert_eq!(
            CollectionProfile::Io.needs(),
            CollectionNeeds {
                disks: true,
                disk_details: true,
                disk_temperatures: true,
                io_wait: true,
                process_io: true,
                processes: true,
                ..CollectionNeeds::default()
            }
        );
        assert_eq!(
            CollectionProfile::Net.needs(),
            CollectionNeeds {
                network: true,
                network_processes: true,
                network_listeners: true,
                processes: true,
                ..CollectionNeeds::default()
            }
        );
        assert_eq!(
            CollectionProfile::Mem.needs(),
            CollectionNeeds {
                memory: true,
                memory_details: true,
                processes: true,
                process_memory: true,
                ..CollectionNeeds::default()
            }
        );
        assert_eq!(
            CollectionProfile::Disks.needs(),
            CollectionNeeds {
                disks: true,
                disk_temperatures: true,
                ..CollectionNeeds::default()
            }
        );
        assert_eq!(
            CollectionProfile::Gpu.needs(),
            CollectionNeeds {
                memory: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                gpu: true,
                gpu_processes: true,
                npu: true,
                ..CollectionNeeds::default()
            }
        );
    }

    #[test]
    fn history_profile_collects_every_recorded_metric() {
        let needs = CollectionProfile::History.needs();
        assert!(needs.cpu);
        assert!(needs.memory);
        assert!(needs.disks);
        assert!(needs.network);
        assert!(needs.gpu);
        assert!(needs.processes);
        assert!(needs.process_io);
    }

    #[test]
    fn collection_needs_union_combines_flags() {
        let a = CollectionNeeds {
            cpu: true,
            cpu_details: true,
            ..CollectionNeeds::default()
        };
        let b = CollectionNeeds {
            disks: true,
            network: true,
            ..CollectionNeeds::default()
        };
        let combined = a.union(b);
        assert!(combined.cpu);
        assert!(combined.cpu_details);
        assert!(combined.disks);
        assert!(combined.network);
        assert!(!combined.memory);
    }

    #[test]
    fn collection_plan_unions_visible_and_recording_needs() {
        let plan = CollectionPlan::new(CollectionProfile::Cpu, true);
        let needs = plan.needs();
        assert!(needs.cpu);
        assert!(needs.cpu_details);
        assert!(needs.cpu_temperatures);
        assert!(needs.process_affinity);
        assert!(needs.disks);
        assert!(needs.network);
        assert!(needs.network_processes);
        assert!(needs.gpu);
        assert!(needs.process_io);

        let non_recording = CollectionPlan::new(CollectionProfile::Cpu, false);
        let non_rec_needs = non_recording.needs();
        assert!(non_rec_needs.cpu_details);
        assert!(!non_rec_needs.disks);
        assert!(!non_rec_needs.network);
    }

    #[test]
    fn collection_plan_respects_selective_recording_mask() {
        let mask = RecordingMask {
            cpu: true,
            mem: true,
            io: false,
            net: false,
            gpu: false,
            npu: false,
        };
        assert_eq!(mask.count(), 2);
        assert_eq!(mask.summary(), "CPU,MEM");

        let plan = CollectionPlan::with_recording_mask(CollectionProfile::Cpu, true, mask);
        let needs = plan.needs();
        assert!(needs.cpu);
        assert!(needs.cpu_details);
        assert!(needs.memory);
        assert!(!needs.disks);
        assert!(!needs.process_io);
        assert!(!needs.network);
        assert!(!needs.network_processes);
        assert!(!needs.gpu);

        let hist_plan = CollectionPlan::with_recording_mask(CollectionProfile::History, true, mask);
        let hist_needs = hist_plan.needs();
        assert!(hist_needs.cpu);
        assert!(hist_needs.memory);
        assert!(!hist_needs.disks);
        assert!(!hist_needs.process_io);
        assert!(!hist_needs.network);
        assert!(!hist_needs.network_processes);
        assert!(!hist_needs.gpu);
    }

    #[test]
    fn last_cpu_from_stat_reads_field_39() {
        let stat = "123 (my app (worker) v2) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 7 38";
        assert_eq!(last_cpu_from_stat(stat), Some(7));
    }

    #[test]
    fn last_cpu_from_stat_requires_close_paren() {
        assert_eq!(last_cpu_from_stat("no parens here"), None);
    }

    #[test]
    fn last_cpu_from_stat_short_line_is_none() {
        assert_eq!(last_cpu_from_stat("1 (init) S 1"), None);
    }

    #[test]
    fn shared_l2_parses_single() {
        assert_eq!(shared_l2_from_list("0,1"), vec![0, 1]);
    }

    #[test]
    fn shared_l2_parses_range() {
        assert_eq!(shared_l2_from_list("8-11"), vec![8, 9, 10, 11]);
    }

    #[test]
    fn shared_l2_parses_mixed() {
        assert_eq!(shared_l2_from_list("4-6,12"), vec![4, 5, 6, 12]);
    }

    #[test]
    fn core_type_classification() {
        assert_eq!(core_type_of(false, 4900), CoreType::P);
        assert_eq!(core_type_of(true, 4400), CoreType::E);
        // Sem freq, so L2 compartilhado decide P vs E.
        assert_eq!(core_type_of(false, 0), CoreType::P);
        assert_eq!(core_type_of(true, 0), CoreType::E);
    }

    #[test]
    fn freq_buckets_sorted_desc_and_deduped() {
        assert_eq!(
            freq_buckets(&[2500, 4900, 4400, 2500]),
            vec![4900, 4400, 2500]
        );
        assert_eq!(freq_buckets(&[0, 0]), Vec::<u64>::new());
    }

    #[test]
    fn core_types_of_splits_three_buckets() {
        // Core Ultra 225H: 4x4900 (P) + 8x4400 (E) + 2x2500 (LPE).
        let freqs = [
            4900, 4900, 4900, 4900, 4400, 4400, 4400, 4400, 4400, 4400, 4400, 4400, 2500, 2500,
        ];
        let buckets = freq_buckets(&freqs);
        assert_eq!(buckets.len(), 3);
        let p_max = buckets[0];
        let lpe_max = buckets[2];
        let types: Vec<CoreType> = freqs
            .iter()
            .map(|&f| {
                if f == lpe_max && buckets.len() >= 3 {
                    CoreType::Lpe
                } else if f == p_max {
                    CoreType::P
                } else {
                    CoreType::E
                }
            })
            .collect();
        assert_eq!(types[0], CoreType::P);
        assert_eq!(types[4], CoreType::E);
        assert_eq!(types[12], CoreType::Lpe);
        assert_eq!(types[13], CoreType::Lpe);
    }

    #[test]
    fn on_cpu_pins_and_returns_value() {
        assert_eq!(on_cpu(0, || Some(CoreType::P)), Some(CoreType::P));
        assert_eq!(on_cpu(0, || None), None);
    }

    #[test]
    fn proc_io_parses_storage_bytes() {
        let raw = "rchar: 1000\nwchar: 2000\nsyscr: 5\nsyscw: 7\nread_bytes: 4096000\nwrite_bytes: 2048000\ncancelled_write_bytes: 0\n";
        assert_eq!(proc_io_from(raw), (4096000, 2048000));
    }

    #[test]
    fn proc_io_missing_fields_are_zero() {
        assert_eq!(proc_io_from("rchar: 1\n"), (0, 0));
    }

    #[test]
    fn stat_iowait_parses_cpu_line() {
        let raw = "cpu  100 50 200 1000 40 30 20 10 5 2\ncpu0 1 2 3 4 5 6 7 8 9 1\n";
        assert_eq!(stat_iowait_from(raw), (40, 1457));
    }

    #[test]
    fn stat_iowait_missing_line_is_zero() {
        assert_eq!(stat_iowait_from("cpu0 1 2 3 4 5\n"), (0, 0));
    }

    #[test]
    fn iowait_uses_counter_delta() {
        assert_eq!(iowait_percent((100, 1_000), (110, 1_100)), 10.0);
        assert_eq!(iowait_percent((200, 2_000), (100, 1_000)), 0.0);
    }

    #[test]
    fn rate_bps_converts_delta() {
        assert_eq!(rate_bps(1_000_000, 2.0), 500_000);
        assert_eq!(rate_bps(100, 0.0), 0);
    }

    #[test]
    fn user_name_of_resolves_root() {
        // root exists in every passwd; the NSS lookup must not return "?".
        let name = user_name_of(0);
        assert_eq!(name, "root");
    }

    #[test]
    fn user_name_of_missing_uid_is_question() {
        // u32::MAX is never a real uid; getpwuid_r must fail -> "?".
        assert_eq!(user_name_of(u32::MAX), "?");
    }
}

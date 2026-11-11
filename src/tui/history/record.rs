//! Folding a live snapshot into the history ring buffer.

use crate::data::cpu::CpuSnapshot;

use super::format::format_local_time;
use super::{HistoryProcess, HistorySample, HistoryState, SampleThermals};

impl HistoryState {
    /// Total read and write rates across every disk in the snapshot.
    fn disk_rates(snap: &CpuSnapshot) -> (u64, u64) {
        let mut read_bps = 0u64;
        let mut write_bps = 0u64;
        for d in &snap.disks {
            read_bps += d.read_bps;
            write_bps += d.write_bps;
        }
        (read_bps, write_bps)
    }

    /// The busiest GPU's utilisation, plus every GPU process across all cards
    /// as `(pid, percent, vram)`.
    fn gpu_usage(snap: &CpuSnapshot) -> (f32, Vec<(u32, f32, u64)>) {
        let mut gpu_pct = 0.0f32;
        let mut gpu_procs: Vec<(u32, f32, u64)> = Vec::new();
        for dev in &snap.gpu.devices {
            if let Some(u) = dev.usage_percent {
                if u > gpu_pct {
                    gpu_pct = u;
                }
            }
            for gp in &dev.processes {
                gpu_procs.push((
                    gp.pid,
                    gp.gpu_percent.unwrap_or(0.0),
                    gp.memory_used_bytes.unwrap_or(0),
                ));
            }
        }
        (gpu_pct, gpu_procs)
    }

    /// The top processes for this sample, with their GPU and socket activity
    /// folded in.
    /// The heaviest processes by the snapshot's own ordering.
    /// `depth` is the picker's setting; `0` means every process the snapshot
    /// carries. The GPU and socket merges below can still push the list past
    /// it, because a process holding the GPU is worth keeping even when it is
    /// not in the busiest N by CPU.
    fn base_processes(
        snap: &CpuSnapshot,
        gpu_procs: &[(u32, f32, u64)],
        depth: usize,
    ) -> Vec<HistoryProcess> {
        let limit = if depth == 0 {
            snap.processes.len()
        } else {
            depth
        };
        let mut procs = Vec::new();
        for p in snap.processes.iter().take(limit) {
            let mut gp_pct = 0.0f32;
            let mut vram = 0u64;
            if let Some(gp) = gpu_procs.iter().find(|(pid, _, _)| *pid == p.pid) {
                gp_pct = gp.1;
                vram = gp.2;
            }
            let np = snap.net.proc_net.iter().find(|n| n.pid == p.pid);
            procs.push(HistoryProcess {
                pid: p.pid,
                start_time: p.start_time,
                name: p.name.clone(),
                cmd: p.cmd.clone(),
                cpu_percent: p.cpu_percent,
                mem_bytes: p.mem_bytes,
                read_bps: p.read_bps,
                write_bps: p.write_bps,
                gpu_percent: gp_pct,
                vram_bytes: vram,
                net_rx_bps: np.map(|n| n.rx_bps).unwrap_or(0),
                net_tx_bps: np.map(|n| n.tx_bps).unwrap_or(0),
                net_rx_bytes: np.map(|n| n.rx_bytes).unwrap_or(0),
                net_tx_bytes: np.map(|n| n.tx_bytes).unwrap_or(0),
                tcp_est: np.map(|n| n.tcp_est).unwrap_or(0),
                udp: np.map(|n| n.udp).unwrap_or(0),
            });
        }
        procs
    }

    /// Adds any GPU process the base list missed, and fills in GPU usage for
    /// the ones already there.
    fn merge_gpu_processes(
        procs: &mut Vec<HistoryProcess>,
        gpu_procs: &[(u32, f32, u64)],
        snap: &CpuSnapshot,
    ) {
        for (g_pid, g_pct, g_vram) in gpu_procs {
            if !procs.iter().any(|p| p.pid == *g_pid) {
                let np = snap.net.proc_net.iter().find(|n| n.pid == *g_pid);
                let p_info = snap.processes.iter().find(|p| p.pid == *g_pid);
                procs.push(HistoryProcess {
                    pid: *g_pid,
                    start_time: p_info.map(|p| p.start_time).unwrap_or(0),
                    name: p_info.map(|p| p.name.clone()).unwrap_or_default(),
                    cmd: p_info.map(|p| p.cmd.clone()).unwrap_or_default(),
                    cpu_percent: p_info.map(|p| p.cpu_percent).unwrap_or(0.0),
                    mem_bytes: p_info.map(|p| p.mem_bytes).unwrap_or(0),
                    read_bps: 0,
                    write_bps: 0,
                    gpu_percent: *g_pct,
                    vram_bytes: *g_vram,
                    net_rx_bps: np.map(|n| n.rx_bps).unwrap_or(0),
                    net_tx_bps: np.map(|n| n.tx_bps).unwrap_or(0),
                    net_rx_bytes: np.map(|n| n.rx_bytes).unwrap_or(0),
                    net_tx_bytes: np.map(|n| n.tx_bytes).unwrap_or(0),
                    tcp_est: np.map(|n| n.tcp_est).unwrap_or(0),
                    udp: np.map(|n| n.udp).unwrap_or(0),
                });
            }
        }
    }

    /// Adds any process with sockets that the base list missed, and fills in
    /// socket counters for the ones already there.
    fn merge_socket_counters(procs: &mut Vec<HistoryProcess>, snap: &CpuSnapshot) {
        for np in &snap.net.proc_net {
            if !procs.iter().any(|p| p.pid == np.pid) {
                let p_info = snap.processes.iter().find(|p| p.pid == np.pid);
                procs.push(HistoryProcess {
                    pid: np.pid,
                    start_time: p_info.map(|p| p.start_time).unwrap_or(0),
                    name: p_info.map(|p| p.name.clone()).unwrap_or_default(),
                    cmd: p_info.map(|p| p.cmd.clone()).unwrap_or_default(),
                    cpu_percent: p_info.map(|p| p.cpu_percent).unwrap_or(0.0),
                    mem_bytes: p_info.map(|p| p.mem_bytes).unwrap_or(0),
                    read_bps: p_info.map(|p| p.read_bps).unwrap_or(0),
                    write_bps: p_info.map(|p| p.write_bps).unwrap_or(0),
                    gpu_percent: 0.0,
                    vram_bytes: 0,
                    net_rx_bps: np.rx_bps,
                    net_tx_bps: np.tx_bps,
                    net_rx_bytes: np.rx_bytes,
                    net_tx_bytes: np.tx_bytes,
                    tcp_est: np.tcp_est,
                    udp: np.udp,
                });
            }
        }
    }

    /// The top processes for this sample, with their GPU and socket activity
    /// folded in.
    fn top_processes(
        snap: &CpuSnapshot,
        gpu_procs: &[(u32, f32, u64)],
        depth: usize,
    ) -> Vec<HistoryProcess> {
        let mut procs = Self::base_processes(snap, gpu_procs, depth);
        Self::merge_gpu_processes(&mut procs, gpu_procs, snap);
        Self::merge_socket_counters(&mut procs, snap);
        procs
    }

    /// The hottest reading of a device list, or `None` when no device in it
    /// has a sensor. The hottest is what throttles; an average hides it.
    fn hottest(readings: impl Iterator<Item = Option<f32>>) -> Option<i16> {
        readings
            .flatten()
            .filter(|c| c.is_finite())
            .fold(None, |acc: Option<f32>, c| {
                Some(acc.map_or(c, |a| a.max(c)))
            })
            .map(|c| c.round() as i16)
    }

    /// Every thermal series of one snapshot, reduced to what a sample keeps.
    pub(super) fn thermals(snap: &CpuSnapshot) -> SampleThermals {
        SampleThermals {
            cpu_temp_c: Self::hottest(std::iter::once(snap.cpu_temp_c)),
            gpu_temp_c: Self::hottest(snap.gpu.devices.iter().map(|d| d.temperature_c)),
            disk_temp_c: Self::hottest(snap.disks.iter().map(|d| d.temp_c)),
            // A fan always answers a number, so an empty list is the only way
            // this machine can mean "no cooler"; a stopped fan is a real 0.
            fan_rpm: snap
                .fans
                .fans
                .iter()
                .map(|f| f.rpm)
                .max()
                .map(|rpm| rpm.min(u32::MAX as u64) as u32),
            per_core_temp_c: snap
                .per_core_temp_c
                .iter()
                .map(|c| Self::hottest(std::iter::once(*c)))
                .collect(),
        }
    }

    pub fn record_snapshot(&mut self, snap: &CpuSnapshot) {
        if !self.recording {
            return;
        }

        let now = std::time::SystemTime::now();
        let timestamp = format_local_time(now);

        let mem_pct = if snap.mem.total > 0 {
            crate::units::percent_of(snap.mem.used, snap.mem.total)
        } else {
            0.0
        };
        let (read_bps, write_bps) = Self::disk_rates(snap);
        let io_mb = (read_bps + write_bps) as f32 / 1_000_000.0;
        let (gpu_pct, gpu_procs) = Self::gpu_usage(snap);
        let procs = Self::top_processes(snap, &gpu_procs, self.record_process_depth);
        let thermals = Self::thermals(snap);

        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }

        let sample = HistorySample {
            timestamp,
            cpu: snap.overall_percent,
            mem: mem_pct,
            io_mb,
            read_bps,
            write_bps,
            gpu: gpu_pct,
            net_rx_bps: snap.net.totals.rx_bps,
            net_tx_bps: snap.net.totals.tx_bps,
            top_procs: procs,
            cpu_temp_c: thermals.cpu_temp_c,
            gpu_temp_c: thermals.gpu_temp_c,
            disk_temp_c: thermals.disk_temp_c,
            fan_rpm: thermals.fan_rpm,
            per_core_temp_c: thermals.per_core_temp_c,
        };

        if self.is_session_recording {
            let mut rec_sample = sample.clone();
            if !self.recording_mask.cpu {
                rec_sample.cpu = 0.0;
            }
            if !self.recording_mask.mem {
                rec_sample.mem = 0.0;
            }
            if !self.recording_mask.io {
                rec_sample.io_mb = 0.0;
                rec_sample.read_bps = 0;
                rec_sample.write_bps = 0;
            }
            if !self.recording_mask.net {
                rec_sample.net_rx_bps = 0;
                rec_sample.net_tx_bps = 0;
            }
            if !self.recording_mask.gpu {
                rec_sample.gpu = 0.0;
            }
            if !self.recording_mask.temp {
                rec_sample.cpu_temp_c = None;
                rec_sample.gpu_temp_c = None;
                rec_sample.disk_temp_c = None;
                rec_sample.fan_rpm = None;
                rec_sample.per_core_temp_c.clear();
            }
            for p in &mut rec_sample.top_procs {
                if !self.recording_mask.cpu {
                    p.cpu_percent = 0.0;
                }
                if !self.recording_mask.mem {
                    p.mem_bytes = 0;
                }
                if !self.recording_mask.io {
                    p.read_bps = 0;
                    p.write_bps = 0;
                }
                if !self.recording_mask.net {
                    p.net_rx_bps = 0;
                    p.net_tx_bps = 0;
                    p.net_rx_bytes = 0;
                    p.net_tx_bytes = 0;
                    p.tcp_est = 0;
                    p.udp = 0;
                }
                if !self.recording_mask.gpu {
                    p.gpu_percent = 0.0;
                    p.vram_bytes = 0;
                }
            }
            self.session_record_buffer.push(rec_sample);
            if self.session_record_buffer.len() >= self.target_record_seconds {
                self.stop_and_save_session();
            }
        }

        self.samples.push_back(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::super::HistoryState;
    use crate::data::cpu::CpuSnapshot;
    use crate::data::cpu::ProcessInfo;
    use crate::data::disk::DiskInfo;
    use crate::data::fan::{FanInfo, FanSnapshot};

    /// A snapshot carrying `n` processes and nothing else, so a depth test is
    /// not also a test of GPU or socket merging.
    fn snapshot_with(n: u32) -> CpuSnapshot {
        CpuSnapshot {
            processes: (0..n)
                .map(|pid| ProcessInfo {
                    pid,
                    name: format!("p{pid}"),
                    ..ProcessInfo::default()
                })
                .collect(),
            ..CpuSnapshot::default()
        }
    }

    #[test]
    fn depth_caps_the_recorded_process_table() {
        let snap = snapshot_with(200);
        assert_eq!(HistoryState::top_processes(&snap, &[], 8).len(), 8);
        assert_eq!(HistoryState::top_processes(&snap, &[], 30).len(), 30);
        assert_eq!(HistoryState::top_processes(&snap, &[], 100).len(), 100);
    }

    /// Depth 0 is the "all" preset, and must not read as an empty table.
    #[test]
    fn depth_zero_keeps_every_process() {
        let snap = snapshot_with(200);
        assert_eq!(HistoryState::top_processes(&snap, &[], 0).len(), 200);
    }

    /// Asking for more than the machine has is not an error.
    #[test]
    fn a_depth_above_the_process_count_keeps_what_there_is() {
        let snap = snapshot_with(12);
        assert_eq!(HistoryState::top_processes(&snap, &[], 100).len(), 12);
    }

    fn thermal_snapshot() -> CpuSnapshot {
        CpuSnapshot {
            cpu_temp_c: Some(61.4),
            per_core_temp_c: vec![Some(52.0), None, Some(68.6)],
            disks: vec![
                DiskInfo {
                    temp_c: Some(33.85),
                    ..DiskInfo::default()
                },
                DiskInfo {
                    temp_c: Some(41.2),
                    ..DiskInfo::default()
                },
                DiskInfo {
                    temp_c: None,
                    ..DiskInfo::default()
                },
            ],
            fans: FanSnapshot {
                fans: vec![
                    FanInfo {
                        rpm: 2542,
                        ..FanInfo::default()
                    },
                    FanInfo {
                        rpm: 3100,
                        ..FanInfo::default()
                    },
                ],
            },
            ..CpuSnapshot::default()
        }
    }

    /// Each device list reduces to its hottest member, because the hottest is
    /// the one that throttles; an average would hide it.
    #[test]
    fn thermals_keep_the_hottest_of_each_device_list() {
        let t = HistoryState::thermals(&thermal_snapshot());

        assert_eq!(t.disk_temp_c, Some(41));
        assert_eq!(t.fan_rpm, Some(3100));
    }

    /// Whole degrees: a tenth of a degree is sensor noise, and halving the
    /// digits matters across thousands of samples.
    #[test]
    fn thermals_round_to_whole_degrees() {
        let t = HistoryState::thermals(&thermal_snapshot());

        assert_eq!(t.cpu_temp_c, Some(61));
        assert_eq!(t.disk_temp_c, Some(41));
        assert_eq!(t.per_core_temp_c, vec![Some(52), None, Some(69)]);
    }

    /// A machine with no sensor must record `None`, never 0: this build runs on
    /// Intel GPUs that report no temperature at all, and 0 degrees reads as a
    /// real cold measurement.
    #[test]
    fn a_missing_sensor_records_none_rather_than_zero() {
        let t = HistoryState::thermals(&CpuSnapshot::default());

        assert_eq!(t.cpu_temp_c, None);
        assert_eq!(t.gpu_temp_c, None);
        assert_eq!(t.disk_temp_c, None);
        assert_eq!(t.fan_rpm, None);
        assert!(t.per_core_temp_c.is_empty());
    }

    /// A disk list where every entry lacks a sensor is the same as no list.
    /// A fan always reports a number, so a stopped fan is a real 0 RPM and an
    /// empty list is the only way to mean "no cooler".
    #[test]
    fn a_device_list_with_no_readings_records_none() {
        let snap = CpuSnapshot {
            disks: vec![DiskInfo::default(), DiskInfo::default()],
            fans: FanSnapshot {
                fans: vec![FanInfo::default()],
            },
            ..CpuSnapshot::default()
        };

        let t = HistoryState::thermals(&snap);
        assert_eq!(t.disk_temp_c, None);
        assert_eq!(t.fan_rpm, Some(0));
    }
}

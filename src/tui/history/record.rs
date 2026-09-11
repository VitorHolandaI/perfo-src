//! Folding a live snapshot into the history ring buffer.

use crate::data::cpu::CpuSnapshot;

use super::format::format_local_time;
use super::{HistoryProcess, HistorySample, HistoryState};

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
    fn base_processes(snap: &CpuSnapshot, gpu_procs: &[(u32, f32, u64)]) -> Vec<HistoryProcess> {
        let mut procs = Vec::new();
        for p in snap.processes.iter().take(30) {
            let mut gp_pct = 0.0f32;
            let mut vram = 0u64;
            if let Some(gp) = gpu_procs.iter().find(|(pid, _, _)| *pid == p.pid) {
                gp_pct = gp.1;
                vram = gp.2;
            }
            let np = snap.net.proc_net.iter().find(|n| n.pid == p.pid);
            procs.push(HistoryProcess {
                pid: p.pid,
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
    fn top_processes(snap: &CpuSnapshot, gpu_procs: &[(u32, f32, u64)]) -> Vec<HistoryProcess> {
        let mut procs = Self::base_processes(snap, gpu_procs);
        Self::merge_gpu_processes(&mut procs, gpu_procs, snap);
        Self::merge_socket_counters(&mut procs, snap);
        procs
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
        let procs = Self::top_processes(snap, &gpu_procs);

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

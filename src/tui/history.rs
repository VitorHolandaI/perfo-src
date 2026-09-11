use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row as TableRow, Table},
    Frame,
};
use serde::{Deserialize, Serialize};

use super::cpu::{self, Pane, Ui};
use crate::data::cpu::{CpuSnapshot, RecordingMask};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HistoryMetric {
    #[default]
    Cpu,
    Mem,
    Io,
    Net,
    Gpu,
}

impl HistoryMetric {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Mem => "MEM",
            Self::Io => "IO",
            Self::Net => "NET",
            Self::Gpu => "GPU",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Mem,
            Self::Mem => Self::Io,
            Self::Io => Self::Net,
            Self::Net => Self::Gpu,
            Self::Gpu => Self::Cpu,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HistorySpan {
    #[default]
    Span2m,
    Span15m,
    Span1h,
    SpanAll,
}

impl HistorySpan {
    pub fn seconds(self, total: usize) -> usize {
        match self {
            Self::Span2m => 120,
            Self::Span15m => 900,
            Self::Span1h => 3600,
            Self::SpanAll => total.max(1),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Span2m => "2m",
            Self::Span15m => "15m",
            Self::Span1h => "1h",
            Self::SpanAll => "ALL",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Span2m => Self::Span15m,
            Self::Span15m => Self::Span1h,
            Self::Span1h => Self::SpanAll,
            Self::SpanAll => Self::Span2m,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct HistoryProcess {
    pub pid: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cmd: String,
    #[serde(default)]
    pub cpu_percent: f32,
    #[serde(default)]
    pub mem_bytes: u64,
    #[serde(default)]
    pub read_bps: u64,
    #[serde(default)]
    pub write_bps: u64,
    #[serde(default)]
    pub gpu_percent: f32,
    #[serde(default)]
    pub vram_bytes: u64,
    #[serde(default)]
    pub net_rx_bps: u64,
    #[serde(default)]
    pub net_tx_bps: u64,
    #[serde(default)]
    pub net_rx_bytes: u64,
    #[serde(default)]
    pub net_tx_bytes: u64,
    #[serde(default)]
    pub tcp_est: u32,
    #[serde(default)]
    pub udp: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HistorySample {
    pub timestamp: String,
    pub cpu: f32,
    pub mem: f32,
    pub io_mb: f32,
    pub read_bps: u64,
    pub write_bps: u64,
    pub gpu: f32,
    #[serde(default)]
    pub net_rx_bps: u64,
    #[serde(default)]
    pub net_tx_bps: u64,
    #[serde(default, alias = "processes")]
    pub top_procs: Vec<HistoryProcess>,
}

pub struct HistoryState {
    pub samples: VecDeque<HistorySample>,
    pub max_samples: usize,
    pub scrub_index: Option<usize>,
    pub metric: HistoryMetric,
    pub span: HistorySpan,
    pub recording: bool,
    pub playing: bool,
    pub export_status: Option<(String, Instant)>,

    // Session recording
    pub is_session_recording: bool,
    pub session_record_buffer: Vec<HistorySample>,
    pub target_record_seconds: usize,
    pub recording_mask: RecordingMask,
    pub record_modal: bool,
    pub record_modal_idx: usize,

    // Saved replay playback
    pub loaded_session_id: Option<String>,
    pub loaded_session_title: Option<String>,
    pub loaded_samples: Vec<HistorySample>,

    // Saved sessions modal
    pub sessions_modal: bool,
    pub saved_recordings: Vec<crate::recordings::RecordingMetadata>,
    pub selected_session_idx: usize,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(3600),
            max_samples: 3600,
            scrub_index: None,
            metric: HistoryMetric::Cpu,
            span: HistorySpan::Span2m,
            recording: true,
            playing: false,
            export_status: None,
            is_session_recording: false,
            session_record_buffer: Vec::new(),
            target_record_seconds: 120,
            recording_mask: RecordingMask::ALL,
            record_modal: false,
            record_modal_idx: 0,
            loaded_session_id: None,
            loaded_session_title: None,
            loaded_samples: Vec::new(),
            sessions_modal: false,
            saved_recordings: Vec::new(),
            selected_session_idx: 0,
        }
    }
}

impl HistoryState {
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

        let mut read_bps = 0u64;
        let mut write_bps = 0u64;
        for d in &snap.disks {
            read_bps += d.read_bps;
            write_bps += d.write_bps;
        }
        let io_mb = (read_bps + write_bps) as f32 / 1_000_000.0;

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

        for (g_pid, g_pct, g_vram) in &gpu_procs {
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

    pub fn sample_count(&self) -> usize {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.len()
        } else {
            self.samples.len()
        }
    }

    pub fn get_sample(&self, idx: usize) -> Option<&HistorySample> {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.get(idx)
        } else {
            self.samples.get(idx)
        }
    }

    pub fn last_sample(&self) -> Option<&HistorySample> {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.last()
        } else {
            self.samples.back()
        }
    }

    pub fn effective_index(&self) -> usize {
        let count = self.sample_count();
        if count == 0 {
            return 0;
        }
        match self.scrub_index {
            Some(idx) => idx.min(count - 1),
            None => count - 1,
        }
    }

    pub fn is_live(&self) -> bool {
        self.loaded_session_id.is_none()
            && (self.scrub_index.is_none()
                || self.effective_index() == self.samples.len().saturating_sub(1))
    }

    pub fn step(&mut self, delta: i32) {
        let count = self.sample_count();
        if count == 0 {
            return;
        }
        self.playing = false;
        let eff = self.effective_index();
        let target = eff as i64 + delta as i64;
        if target < 0 {
            self.scrub_index = Some(0);
        } else if target >= (count - 1) as i64 {
            if self.loaded_session_id.is_some() {
                self.scrub_index = Some(count - 1);
            } else {
                self.scrub_index = None;
            }
        } else {
            self.scrub_index = Some(target as usize);
        }
    }

    pub fn jump_step(&self) -> usize {
        match self.span {
            HistorySpan::Span2m => 10,
            HistorySpan::Span15m => 30,
            HistorySpan::Span1h => 60,
            HistorySpan::SpanAll => 300,
        }
    }

    pub fn jump(&mut self, direction: i32) {
        let step = self.jump_step() as i32 * (if direction < 0 { -1 } else { 1 });
        self.step(step);
    }

    pub fn toggle_playback(&mut self) {
        let count = self.sample_count();
        if count == 0 {
            return;
        }
        if self.playing {
            self.playing = false;
        } else {
            if self.is_live() && count > 0 {
                let span = self.span.seconds(count);
                self.scrub_index = Some(count.saturating_sub(span));
            }
            self.playing = true;
        }
    }

    pub fn advance_playback(&mut self) {
        let count = self.sample_count();
        if !self.playing || count == 0 {
            return;
        }
        let eff = self.effective_index();
        if eff + 1 >= count {
            if self.loaded_session_id.is_some() {
                self.scrub_index = Some(count - 1);
            } else {
                self.scrub_index = None;
            }
            self.playing = false;
        } else {
            self.scrub_index = Some(eff + 1);
        }
    }

    pub fn jump_to_live(&mut self) {
        self.loaded_session_id = None;
        self.loaded_session_title = None;
        self.loaded_samples.clear();
        self.scrub_index = None;
        self.playing = false;
    }

    pub fn stop_and_save_session(&mut self) {
        if !self.is_session_recording || self.session_record_buffer.is_empty() {
            self.is_session_recording = false;
            self.session_record_buffer.clear();
            return;
        }
        self.is_session_recording = false;
        let dur = self.session_record_buffer.len() as u64;
        let val = serde_json::to_value(&self.session_record_buffer)
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        let focus = self.recording_mask.summary();
        match crate::recordings::save_session_data(val, dur, &focus) {
            Ok(meta) => {
                let rec_id = meta.id.clone();
                let dur_str = meta.duration.clone();
                let _ = self.load_session(&rec_id);
                if !self.loaded_samples.is_empty() {
                    self.scrub_index = Some(self.loaded_samples.len() - 1);
                }
                self.export_status = Some((
                    format!("Saved session: {} ({}) [{}]", rec_id, dur_str, focus),
                    Instant::now(),
                ));
            }
            Err(e) => {
                self.export_status = Some((format!("Failed to save: {}", e), Instant::now()));
            }
        }
        self.session_record_buffer.clear();
    }

    pub fn open_record_modal(&mut self) {
        self.record_modal = true;
        self.record_modal_idx = 0;
    }

    pub fn close_record_modal(&mut self) {
        self.record_modal = false;
    }

    pub fn record_modal_next(&mut self) {
        self.record_modal_idx = (self.record_modal_idx + 1) % 8;
    }

    pub fn record_modal_prev(&mut self) {
        self.record_modal_idx = if self.record_modal_idx == 0 {
            7
        } else {
            self.record_modal_idx - 1
        };
    }

    pub fn toggle_record_mask_item(&mut self, idx: usize) {
        let count = self.recording_mask.count();
        let target = match idx {
            0 => &mut self.recording_mask.cpu,
            1 => &mut self.recording_mask.mem,
            2 => &mut self.recording_mask.io,
            3 => &mut self.recording_mask.net,
            4 => &mut self.recording_mask.gpu,
            5 => &mut self.recording_mask.npu,
            _ => return,
        };
        if !*target || count > 1 {
            *target = !*target;
        }
    }

    pub fn start_session_recording(&mut self) {
        self.record_modal = false;
        self.is_session_recording = true;
        self.session_record_buffer.clear();
        let summary = self.recording_mask.summary();
        self.export_status = Some((
            format!("Recording session [{}] started...", summary),
            Instant::now(),
        ));
    }

    pub fn toggle_session_recording(&mut self) {
        if self.is_session_recording {
            self.stop_and_save_session();
        } else {
            self.open_record_modal();
        }
    }

    pub fn open_sessions_modal(&mut self) {
        self.refresh_saved_recordings();
        self.selected_session_idx = 0;
        self.sessions_modal = true;
    }

    pub fn refresh_saved_recordings(&mut self) {
        self.saved_recordings = crate::recordings::get_recordings_list();
        if self.saved_recordings.is_empty() {
            self.selected_session_idx = 0;
        } else if self.selected_session_idx >= self.saved_recordings.len() {
            self.selected_session_idx = self.saved_recordings.len() - 1;
        }
    }

    pub fn close_sessions_modal(&mut self) {
        self.sessions_modal = false;
    }

    pub fn modal_next(&mut self) {
        if !self.saved_recordings.is_empty() {
            self.selected_session_idx =
                (self.selected_session_idx + 1).min(self.saved_recordings.len() - 1);
        }
    }

    pub fn modal_prev(&mut self) {
        if self.selected_session_idx > 0 {
            self.selected_session_idx -= 1;
        }
    }

    pub fn modal_load_selected(&mut self) {
        if let Some(meta) = self.saved_recordings.get(self.selected_session_idx) {
            let id = meta.id.clone();
            if let Err(e) = self.load_session(&id) {
                self.export_status = Some((format!("Load error: {}", e), Instant::now()));
            }
        }
    }

    pub fn modal_delete_selected(&mut self) {
        if let Some(meta) = self.saved_recordings.get(self.selected_session_idx) {
            let id = meta.id.clone();
            let _ = crate::recordings::delete_recording_by_id(&id);
            self.export_status = Some((format!("Deleted session {}", id), Instant::now()));
            self.refresh_saved_recordings();
        }
    }

    pub fn load_session(&mut self, id: &str) -> Result<(), String> {
        let payload = crate::recordings::load_recording_payload(id).map_err(|e| e.to_string())?;
        let samples: Vec<HistorySample> = serde_json::from_value(payload.samples)
            .map_err(|e| format!("Failed to parse samples: {}", e))?;
        if samples.is_empty() {
            return Err("Session has no samples".to_string());
        }
        let title = format!("{} ({})", payload.id, payload.duration_label);
        self.loaded_session_id = Some(payload.id.clone());
        self.loaded_session_title = Some(title);
        self.loaded_samples = samples;
        self.scrub_index = Some(0);
        self.playing = false;
        self.sessions_modal = false;
        self.export_status = Some((format!("Loaded replay {}", id), Instant::now()));
        Ok(())
    }

    pub fn export(&mut self) {
        if self.samples.is_empty() {
            self.export_status = Some(("No history to export".to_string(), Instant::now()));
            return;
        }

        let now = std::time::SystemTime::now();
        let date_str = format_local_datetime(now);
        let base_name = format!("perfo-history-{}", date_str);

        let home_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        let txt_path = home_dir.join(format!("{}.txt", base_name));
        let json_path = home_dir.join(format!("{}.json", base_name));

        let text_content = self.generate_export_text(&date_str);
        let json_content = self.generate_export_json(&date_str);

        let res_txt = fs::write(&txt_path, text_content);
        let res_json = fs::write(&json_path, json_content);

        if res_txt.is_ok() && res_json.is_ok() {
            self.export_status = Some((
                format!("Saved: ~/{}.{{txt,json}}", base_name),
                Instant::now(),
            ));
        } else {
            self.export_status = Some(("Export failed".to_string(), Instant::now()));
        }
    }

    fn generate_export_text(&self, date_str: &str) -> String {
        let mut lines = Vec::new();
        let border =
            "================================================================================";
        let sub_border =
            "--------------------------------------------------------------------------------";

        lines.push(border.to_string());
        lines.push("                   PERFO - SYSTEM HISTORY & ANALYSIS REPORT".to_string());
        lines.push(border.to_string());
        lines.push(format!("Generated at         : {}", date_str));
        lines.push(format!("Active Metric Focus  : {}", self.metric.label()));
        lines.push(format!(
            "Timeline View Span   : {} ({}s)",
            self.span.label(),
            self.span.seconds(self.sample_count())
        ));
        lines.push(format!(
            "Total Recorded Time  : {}s ({} samples)",
            self.sample_count(),
            self.sample_count()
        ));
        lines.push(format!(
            "Current View State   : {}",
            if self.is_live() { "LIVE" } else { "SCRUBBED" }
        ));
        lines.push(String::new());

        let eff = self.effective_index();
        let sample = self.get_sample(eff);
        lines.push(border.to_string());
        lines.push(format!(
            "                   1. SNAPSHOT AT SELECTED TIMING ({})",
            sample.map(|s| s.timestamp.as_str()).unwrap_or("--")
        ));
        lines.push(border.to_string());

        if let Some(s) = sample {
            lines.push(format!("Overall CPU Usage    : {:.1}%", s.cpu));
            lines.push(format!("Overall Memory Usage : {:.1}%", s.mem));
            lines.push(format!("Total Disk I/O Rate  : {:.1} MB/s", s.io_mb));
            lines.push(format!(
                "Total Network Rate   : {} (RX: {}, TX: {})",
                cpu::human_bytes(s.net_rx_bps + s.net_tx_bps),
                cpu::human_bytes(s.net_rx_bps),
                cpu::human_bytes(s.net_tx_bps)
            ));
            lines.push(format!("GPU Usage            : {:.1}%", s.gpu));
            lines.push(String::new());
            lines.push("Active Processes at this timing:".to_string());
            if self.metric == HistoryMetric::Net {
                lines.push(format!(
                    "{:<8} | {:<16} | {:>12} | {:>12} | {:<22} | COMMAND",
                    "PID", "PROCESS", "IN (RX)", "OUT (TX)", "TOTAL / CONNS"
                ));
                lines.push(sub_border.to_string());

                let mut procs = s.top_procs.clone();
                self.sort_processes(&mut procs);
                for p in procs.iter().take(15) {
                    let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
                    let rx_str = format_proc_net_io(p.net_rx_bps, p.net_rx_bytes);
                    let tx_str = format_proc_net_io(p.net_tx_bps, p.net_tx_bytes);
                    let conns_str = format_proc_conns(p);
                    lines.push(format!(
                        "{:<8} | {:<16} | {:>12} | {:>12} | {:<22} | {}",
                        p.pid, p_name, rx_str, tx_str, conns_str, p.cmd
                    ));
                }
            } else {
                lines.push(format!(
                    "{:<8} | {:>8} | {:>10} | {:<18} | COMMAND",
                    "PID",
                    self.metric.label(),
                    if self.metric == HistoryMetric::Gpu {
                        "VRAM"
                    } else {
                        "RAM"
                    },
                    "PROCESS"
                ));
                lines.push(sub_border.to_string());

                let mut procs = s.top_procs.clone();
                self.sort_processes(&mut procs);
                for p in procs.iter().take(15) {
                    let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
                    let primary = self.metric_cell_text(p);
                    let secondary = if self.metric == HistoryMetric::Gpu {
                        cpu::human_bytes(p.vram_bytes)
                    } else {
                        cpu::human_bytes(p.mem_bytes)
                    };
                    lines.push(format!(
                        "{:<8} | {:>8} | {:>10} | {:<18} | {}",
                        p.pid, primary, secondary, p_name, p.cmd
                    ));
                }
            }
        }
        lines.push(String::new());

        // Peaks & summary
        lines.push(border.to_string());
        lines.push("                   2. TIMELINE METRICS & PEAKS SUMMARY".to_string());
        lines.push(border.to_string());

        let mut peak_cpu = 0.0f32;
        let mut peak_mem = 0.0f32;
        let mut peak_io = 0.0f32;
        let mut peak_net = 0u64;
        let mut peak_gpu = 0.0f32;
        let mut total_cpu = 0.0f64;
        let mut total_mem = 0.0f64;

        let count = self.sample_count();
        for idx in 0..count {
            if let Some(s) = self.get_sample(idx) {
                total_cpu += s.cpu as f64;
                total_mem += s.mem as f64;
                if s.cpu > peak_cpu {
                    peak_cpu = s.cpu;
                }
                if s.mem > peak_mem {
                    peak_mem = s.mem;
                }
                if s.io_mb > peak_io {
                    peak_io = s.io_mb;
                }
                let net_tot = s.net_rx_bps + s.net_tx_bps;
                if net_tot > peak_net {
                    peak_net = net_tot;
                }
                if s.gpu > peak_gpu {
                    peak_gpu = s.gpu;
                }
            }
        }

        let cnt_f64 = count.max(1) as f64;
        lines.push(format!("Peak CPU Usage       : {:.1}%", peak_cpu));
        lines.push(format!("Peak Memory Usage    : {:.1}%", peak_mem));
        lines.push(format!("Peak Disk I/O Rate   : {:.1} MB/s", peak_io));
        lines.push(format!(
            "Peak Network Rate    : {}/s",
            cpu::human_bytes(peak_net)
        ));
        lines.push(format!("Peak GPU Usage       : {:.1}%", peak_gpu));
        lines.push(format!(
            "Average CPU Usage    : {:.1}%",
            total_cpu / cnt_f64
        ));
        lines.push(format!(
            "Average Memory Usage : {:.1}%",
            total_mem / cnt_f64
        ));
        lines.push(String::new());

        lines.push(border.to_string());
        lines.push("End of Perfo History Report".to_string());
        lines.push(String::new());
        lines.join("\n")
    }

    fn generate_export_json(&self, date_str: &str) -> String {
        #[derive(Serialize)]
        struct ExportJson<'a> {
            version: &'static str,
            generator: &'static str,
            generated_at: &'a str,
            metric_focus: &'static str,
            zoom_span: &'static str,
            total_samples: usize,
            is_live: bool,
            selected_sample: Option<&'a HistorySample>,
            samples: Vec<&'a HistorySample>,
        }

        let eff = self.effective_index();
        let selected = self.get_sample(eff);
        let count = self.sample_count();
        let step = (count / 1000).max(1);
        let mut sampled: Vec<&HistorySample> = Vec::new();
        for idx in (0..count).step_by(step) {
            if let Some(s) = self.get_sample(idx) {
                sampled.push(s);
            }
        }

        let export = ExportJson {
            version: "1.0",
            generator: "perfo",
            generated_at: date_str,
            metric_focus: self.metric.label(),
            zoom_span: self.span.label(),
            total_samples: count,
            is_live: self.is_live(),
            selected_sample: selected,
            samples: sampled,
        };

        serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".to_string())
    }

    fn sort_processes(&self, list: &mut Vec<HistoryProcess>) {
        match self.metric {
            HistoryMetric::Gpu => {
                list.retain(|p| p.gpu_percent > 0.0 || p.vram_bytes > 0);
                list.sort_by(|a, b| {
                    b.gpu_percent
                        .partial_cmp(&a.gpu_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.vram_bytes.cmp(&a.vram_bytes))
                });
            }
            HistoryMetric::Io => {
                list.sort_by_key(|p| std::cmp::Reverse(p.read_bps + p.write_bps));
            }
            HistoryMetric::Mem => {
                list.sort_by_key(|p| std::cmp::Reverse(p.mem_bytes));
            }
            HistoryMetric::Cpu => {
                list.sort_by(|a, b| {
                    b.cpu_percent
                        .partial_cmp(&a.cpu_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            HistoryMetric::Net => {
                list.retain(|p| {
                    p.net_rx_bps > 0
                        || p.net_tx_bps > 0
                        || p.net_rx_bytes > 0
                        || p.net_tx_bytes > 0
                        || p.tcp_est > 0
                        || p.udp > 0
                });
                list.sort_by_key(|p| {
                    std::cmp::Reverse(
                        (p.net_rx_bps + p.net_tx_bps) * 1000
                            + (p.net_rx_bytes + p.net_tx_bytes)
                            + (p.tcp_est as u64 * 100)
                            + p.udp as u64,
                    )
                });
            }
        }
    }

    fn metric_cell_text(&self, proc: &HistoryProcess) -> String {
        match self.metric {
            HistoryMetric::Mem => cpu::human_bytes(proc.mem_bytes),
            HistoryMetric::Gpu => {
                if proc.gpu_percent > 0.0 {
                    format!("{:.0}%", proc.gpu_percent)
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Io => {
                let total = proc.read_bps + proc.write_bps;
                if total > 0 {
                    format!("{}/s", cpu::human_bytes(total))
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Net => {
                let total_bps = proc.net_rx_bps + proc.net_tx_bps;
                let total_bytes = proc.net_rx_bytes + proc.net_tx_bytes;
                if total_bps > 0 {
                    format!("{}/s", cpu::human_bytes(total_bps))
                } else if total_bytes > 0 {
                    cpu::human_bytes(total_bytes)
                } else {
                    "--".to_string()
                }
            }
            HistoryMetric::Cpu => format!("{:.0}%", proc.cpu_percent),
        }
    }
}

pub fn draw_history(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let outer = cpu::block("7:HISTORY ANALYSIS", ui.pane == Pane::History, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    if inner.height < 10 || inner.width < 30 {
        return;
    }

    let [controls_area, sparkline_area, stats_area, table_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(inner);

    draw_controls(frame, controls_area, ui, state);
    draw_timeline_chart(frame, sparkline_area, ui, state);
    draw_stats_box(frame, stats_area, ui, state);
    draw_processes_table(frame, table_area, ui, state);

    if state.sessions_modal {
        draw_sessions_modal(frame, area, ui, state);
    } else if state.record_modal {
        draw_record_modal(frame, area, ui, state);
    }
}

fn draw_controls(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let rec_tag = if state.is_session_recording {
        let cur = state.session_record_buffer.len();
        let tot = state.target_record_seconds;
        let c_m = cur / 60;
        let c_s = cur % 60;
        let t_m = tot / 60;
        let t_s = tot % 60;
        let summary = state.recording_mask.summary();
        let tag_text = if summary == "ALL" {
            format!("● REC {:02}:{:02}/{:02}:{:02}", c_m, c_s, t_m, t_s)
        } else {
            format!(
                "● REC {:02}:{:02}/{:02}:{:02} [{}]",
                c_m, c_s, t_m, t_s, summary
            )
        };
        Span::styled(
            tag_text,
            Style::default()
                .fg(ui.theme.red)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(title) = &state.loaded_session_title {
        Span::styled(
            format!("▶ REPLAY: {}", title),
            Style::default()
                .fg(ui.theme.yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if state.recording {
        Span::styled(
            "● REC",
            Style::default()
                .fg(ui.theme.red)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("⏸ PAUSED", Style::default().fg(ui.theme.muted))
    };

    let play_tag = if state.playing {
        Span::styled(
            " [PLAYING]",
            Style::default()
                .fg(ui.theme.yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };

    let eff = state.effective_index();
    let is_live = state.is_live();
    let total = state.sample_count();
    let span_secs = state.span.seconds(total);
    let start_idx = total.saturating_sub(span_secs);
    let total_span = span_secs.min(total).max(1);
    let elapsed = if is_live {
        total_span
    } else {
        let span_samples = total.saturating_sub(1).saturating_sub(start_idx).max(1);
        let ratio = (eff.saturating_sub(start_idx) as f64) / (span_samples as f64);
        ((ratio * total_span as f64).round() as usize).min(total_span)
    };
    let time_str = state
        .get_sample(eff)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let time_mode = if is_live {
        Span::styled(
            format!(" LIVE [+{}s] ({})", total_span, time_str),
            Style::default()
                .fg(ui.theme.green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" +{}s ({})", elapsed, time_str),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    };

    let metric_pill = |m: HistoryMetric| {
        let label = m.label();
        if state.metric == m {
            Span::styled(
                format!(" [{label}] "),
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("  {label}  "), Style::default().fg(ui.theme.fg))
        }
    };

    let span_pill = |sp: HistorySpan| {
        let label = sp.label();
        if state.span == sp {
            Span::styled(
                format!(" [{label}] "),
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("  {label}  "), Style::default().fg(ui.theme.muted))
        }
    };

    let el_m = elapsed / 60;
    let el_s = elapsed % 60;
    let tot_m = total_span / 60;
    let tot_s = total_span % 60;

    let timer_tag = Span::styled(
        format!(" ⏱ {:02}:{:02}/{:02}:{:02} ", el_m, el_s, tot_m, tot_s),
        Style::default()
            .fg(Color::Black)
            .bg(ui.theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    let line1 = Line::from(vec![
        Span::styled("STATUS: ", Style::default().fg(ui.theme.muted)),
        rec_tag,
        play_tag,
        time_mode,
        Span::raw(" "),
        timer_tag,
        Span::raw("  │  "),
        Span::styled("METRIC: ", Style::default().fg(ui.theme.muted)),
        metric_pill(HistoryMetric::Cpu),
        metric_pill(HistoryMetric::Mem),
        metric_pill(HistoryMetric::Io),
        metric_pill(HistoryMetric::Net),
        metric_pill(HistoryMetric::Gpu),
        Span::raw("  │  "),
        Span::styled("SPAN: ", Style::default().fg(ui.theme.muted)),
        span_pill(HistorySpan::Span2m),
        span_pill(HistorySpan::Span15m),
        span_pill(HistorySpan::Span1h),
        span_pill(HistorySpan::SpanAll),
    ]);

    let export_text = if let Some((msg, inst)) = &state.export_status {
        if inst.elapsed().as_secs() < 4 {
            Span::styled(
                format!("  [{msg}]"),
                Style::default()
                    .fg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        }
    } else {
        Span::raw("")
    };

    let line2 = Line::from(vec![
        Span::styled("KEYS: ", Style::default().fg(ui.theme.muted)),
        Span::styled("< > / ← →", Style::default().fg(ui.theme.yellow)),
        Span::raw(" step  "),
        Span::styled("[ ]", Style::default().fg(ui.theme.yellow)),
        Span::raw(" jump  "),
        Span::styled("Space", Style::default().fg(ui.theme.yellow)),
        Span::raw(" play  "),
        Span::styled("0/L", Style::default().fg(ui.theme.yellow)),
        Span::raw(" live  "),
        Span::styled("Tab", Style::default().fg(ui.theme.yellow)),
        Span::raw(" metric  "),
        Span::styled("z", Style::default().fg(ui.theme.yellow)),
        Span::raw(" span  "),
        Span::styled("r", Style::default().fg(ui.theme.yellow)),
        Span::raw(" rec  "),
        Span::styled("s", Style::default().fg(ui.theme.yellow)),
        Span::raw(" sessions  "),
        Span::styled("e", Style::default().fg(ui.theme.yellow)),
        Span::raw(" export"),
        export_text,
    ]);

    let par = Paragraph::new(vec![line1, line2]);
    frame.render_widget(par, area);
}

fn draw_timeline_chart(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.muted))
        .title(format!(" TIMELINE GRAPH ({}) ", state.metric.label()));
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let total = state.sample_count();
    let span_secs = state.span.seconds(total);
    let start_idx = total.saturating_sub(span_secs);
    let visible_count = total.saturating_sub(start_idx);

    if visible_count == 0 {
        let msg = Paragraph::new("Collecting history samples...")
            .style(Style::default().fg(ui.theme.muted));
        frame.render_widget(msg, inner);
        return;
    }

    let w = inner.width as usize;
    let eff = state.effective_index();

    let mut cursor_chars = vec![' '; w];
    let mut bar_spans = Vec::with_capacity(w);

    let step = (visible_count as f64) / (w as f64);
    let cursor_pos = if eff >= start_idx && visible_count > 0 {
        let rel = eff - start_idx;
        ((rel as f64 / visible_count as f64) * (w as f64 - 1.0)).round() as usize
    } else {
        w.saturating_sub(1)
    };

    if cursor_pos < w {
        cursor_chars[cursor_pos] = '▼';
    }

    let max_metric_val = match state.metric {
        HistoryMetric::Cpu | HistoryMetric::Mem | HistoryMetric::Gpu => 100.0f32,
        HistoryMetric::Io => {
            let mut m = 10.0f32;
            for idx in 0..total {
                if let Some(s) = state.get_sample(idx) {
                    if s.io_mb > m {
                        m = s.io_mb;
                    }
                }
            }
            m
        }
        HistoryMetric::Net => {
            let mut m = 100_000.0f32; // minimum scale 100 KB/s
            for idx in 0..total {
                if let Some(s) = state.get_sample(idx) {
                    let v = (s.net_rx_bps + s.net_tx_bps) as f32;
                    if v > m {
                        m = v;
                    }
                }
            }
            m
        }
    };

    const GLYPHS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    for col in 0..w {
        let b_start = (start_idx as f64 + col as f64 * step).floor() as usize;
        let b_end = (start_idx as f64 + (col + 1) as f64 * step).ceil() as usize;
        let b_end = b_end.min(total).max(b_start + 1);

        let mut peak = 0.0f32;
        for idx in b_start..b_end.min(total) {
            if let Some(s) = state.get_sample(idx) {
                let v = match state.metric {
                    HistoryMetric::Cpu => s.cpu,
                    HistoryMetric::Mem => s.mem,
                    HistoryMetric::Io => s.io_mb,
                    HistoryMetric::Net => (s.net_rx_bps + s.net_tx_bps) as f32,
                    HistoryMetric::Gpu => s.gpu,
                };
                if v > peak {
                    peak = v;
                }
            }
        }

        let ratio = (peak / max_metric_val).clamp(0.0, 1.0);
        let g_idx = ((ratio * 7.0).round() as usize).min(7);
        let ch = GLYPHS[g_idx];

        let is_cur = col == cursor_pos;
        let style = if is_cur {
            Style::default()
                .fg(Color::Black)
                .bg(ui.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if peak >= 80.0
            && state.metric != HistoryMetric::Io
            && state.metric != HistoryMetric::Net
        {
            Style::default().fg(ui.theme.red)
        } else if peak >= 50.0
            && state.metric != HistoryMetric::Io
            && state.metric != HistoryMetric::Net
        {
            Style::default().fg(ui.theme.yellow)
        } else {
            Style::default().fg(ui.theme.accent)
        };

        bar_spans.push(Span::styled(ch.to_string(), style));
    }

    let cursor_line = Line::from(Span::styled(
        cursor_chars.into_iter().collect::<String>(),
        Style::default()
            .fg(ui.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    let bars_line = Line::from(bar_spans);

    let start_time = state
        .get_sample(start_idx)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let end_time = state
        .last_sample()
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let cur_time = state
        .get_sample(eff)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");

    let mut ruler_spans = Vec::with_capacity(w);
    for col in 0..w {
        if col == cursor_pos {
            ruler_spans.push(Span::styled(
                "▲",
                Style::default()
                    .fg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if col == 0 || col == w.saturating_sub(1) {
            ruler_spans.push(Span::styled("|", Style::default().fg(ui.theme.fg)));
        } else if col == w / 4 || col == w / 2 || col == (3 * w) / 4 {
            ruler_spans.push(Span::styled("+", Style::default().fg(ui.theme.muted)));
        } else {
            ruler_spans.push(Span::styled("-", Style::default().fg(ui.theme.muted)));
        }
    }
    let ruler_line = Line::from(ruler_spans);

    let total_span = span_secs.min(total).max(1);
    let elapsed = if state.is_live() {
        total_span
    } else {
        let span_samples = total.saturating_sub(1).saturating_sub(start_idx).max(1);
        let ratio = (eff.saturating_sub(start_idx) as f64) / (span_samples as f64);
        ((ratio * total_span as f64).round() as usize).min(total_span)
    };
    let el_m = elapsed / 60;
    let el_s = elapsed % 60;
    let tot_m = total_span / 60;
    let tot_s = total_span % 60;

    let cur_str = if state.is_live() {
        format!("+{}s LIVE", elapsed)
    } else {
        format!("+{}s", elapsed)
    };

    let time_axis_line = Line::from(vec![
        Span::styled(
            format!("{:<12}", start_time),
            Style::default().fg(ui.theme.muted),
        ),
        Span::styled(
            format!(
                "{:^width$}",
                format!("▲ {} ({})", cur_time, cur_str),
                width = w.saturating_sub(24)
            ),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>12}", end_time),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    let timer_axis_line = Line::from(vec![
        Span::styled(
            format!("{:<12}", "+0s"),
            Style::default().fg(ui.theme.muted),
        ),
        Span::styled(
            format!(
                "{:^width$}",
                format!("[⏱ {:02}:{:02} / {:02}:{:02}]", el_m, el_s, tot_m, tot_s),
                width = w.saturating_sub(24)
            ),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("{:>12}", format!("+{}s LIVE", total_span)),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    let par = Paragraph::new(vec![
        cursor_line,
        bars_line,
        ruler_line,
        time_axis_line,
        timer_axis_line,
    ]);
    frame.render_widget(par, inner);
}

fn draw_stats_box(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let eff = state.effective_index();
    let sample = state.get_sample(eff);

    let (cur_cpu, cur_mem, cur_io, cur_net, cur_gpu) = match sample {
        Some(s) => (s.cpu, s.mem, s.io_mb, s.net_rx_bps + s.net_tx_bps, s.gpu),
        None => (0.0, 0.0, 0.0, 0, 0.0),
    };

    let mut peak_cpu = 0.0f32;
    let mut peak_mem = 0.0f32;
    let mut peak_io = 0.0f32;
    let mut peak_net = 0u64;
    let mut peak_gpu = 0.0f32;
    let mut total_cpu = 0.0f64;
    let mut total_mem = 0.0f64;

    let count = state.sample_count();
    for idx in 0..count {
        if let Some(s) = state.get_sample(idx) {
            total_cpu += s.cpu as f64;
            total_mem += s.mem as f64;
            if s.cpu > peak_cpu {
                peak_cpu = s.cpu;
            }
            if s.mem > peak_mem {
                peak_mem = s.mem;
            }
            if s.io_mb > peak_io {
                peak_io = s.io_mb;
            }
            let net_tot = s.net_rx_bps + s.net_tx_bps;
            if net_tot > peak_net {
                peak_net = net_tot;
            }
            if s.gpu > peak_gpu {
                peak_gpu = s.gpu;
            }
        }
    }

    let cnt_f64 = count.max(1) as f64;
    let avg_cpu = total_cpu / cnt_f64;
    let avg_mem = total_mem / cnt_f64;

    let line1 = Line::from(vec![
        Span::styled("SAMPLE: ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:>4.1}%  ", cur_cpu),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("MEM {:>4.1}%  ", cur_mem),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("IO {:>5.1} MB/s  ", cur_io),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("NET {:>8}/s  ", cpu::human_bytes(cur_net)),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("GPU {:>4.1}%", cur_gpu),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::styled("PEAKS : ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:.1}%  ", peak_cpu),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("MEM {:.1}%  ", peak_mem),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("IO {:.1} MB/s  ", peak_io),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("NET {}/s  ", cpu::human_bytes(peak_net)),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("GPU {:.1}%  │  ", peak_gpu),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled("AVG: ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:.1}%  ", avg_cpu),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("MEM {:.1}%  ", avg_mem),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("({} samples)", count),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    let par = Paragraph::new(vec![line1, line2]);
    frame.render_widget(par, area);
}

fn draw_processes_table(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let eff = state.effective_index();
    let sample = state.get_sample(eff);

    let mut procs = sample.map(|s| s.top_procs.clone()).unwrap_or_default();
    state.sort_processes(&mut procs);

    let title = format!(
        " ACTIVE PROCESSES AT SELECTED TIMING ({}) ",
        sample.map(|s| s.timestamp.as_str()).unwrap_or("--")
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.muted))
        .title(title);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);

    let max_rows = inner.height.saturating_sub(1) as usize;

    if state.metric == HistoryMetric::Net {
        let header = TableRow::new(vec![
            Span::styled("PID", Style::default().fg(ui.theme.muted)),
            Span::styled("PROCESS", Style::default().fg(ui.theme.muted)),
            Span::styled("IN (RX)", Style::default().fg(ui.theme.muted)),
            Span::styled("OUT (TX)", Style::default().fg(ui.theme.muted)),
            Span::styled("TOTAL / CONNS", Style::default().fg(ui.theme.muted)),
            Span::styled("COMMAND", Style::default().fg(ui.theme.muted)),
        ]);

        let rows: Vec<TableRow> = procs
            .iter()
            .take(max_rows)
            .map(|p| {
                let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
                let rx_str = format_proc_net_io(p.net_rx_bps, p.net_rx_bytes);
                let tx_str = format_proc_net_io(p.net_tx_bps, p.net_tx_bytes);
                let conns_str = format_proc_conns(p);
                TableRow::new(vec![
                    Span::styled(format!("{:<7}", p.pid), Style::default().fg(ui.theme.fg)),
                    Span::styled(
                        p_name,
                        Style::default()
                            .fg(ui.theme.fg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(rx_str, Style::default().fg(ui.theme.accent)),
                    Span::styled(tx_str, Style::default().fg(ui.theme.accent)),
                    Span::styled(conns_str, Style::default().fg(ui.theme.fg)),
                    Span::styled(p.cmd.clone(), Style::default().fg(ui.theme.muted)),
                ])
            })
            .collect();

        if rows.is_empty() {
            let p = Paragraph::new("No network socket activity recorded for this sample")
                .style(Style::default().fg(ui.theme.muted));
            frame.render_widget(p, inner);
            return;
        }

        let widths = [
            Constraint::Length(8),
            Constraint::Length(16),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(22),
            Constraint::Min(0),
        ];

        let table = Table::new(rows, widths).header(header);
        frame.render_widget(table, inner);
        return;
    }

    let primary_hdr = match state.metric {
        HistoryMetric::Cpu => "CPU%",
        HistoryMetric::Mem => "MEM%",
        HistoryMetric::Io => "IO RATE",
        HistoryMetric::Gpu => "GPU%",
        HistoryMetric::Net => "NET",
    };
    let secondary_hdr = match state.metric {
        HistoryMetric::Gpu => "VRAM",
        _ => "RAM",
    };

    let header = TableRow::new(vec![
        Span::styled("PID", Style::default().fg(ui.theme.muted)),
        Span::styled("PROCESS", Style::default().fg(ui.theme.muted)),
        Span::styled(primary_hdr, Style::default().fg(ui.theme.muted)),
        Span::styled(secondary_hdr, Style::default().fg(ui.theme.muted)),
        Span::styled("COMMAND", Style::default().fg(ui.theme.muted)),
    ]);

    let rows: Vec<TableRow> = procs
        .iter()
        .take(max_rows)
        .map(|p| {
            let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
            let primary = state.metric_cell_text(p);
            let secondary = if state.metric == HistoryMetric::Gpu {
                cpu::human_bytes(p.vram_bytes)
            } else {
                cpu::human_bytes(p.mem_bytes)
            };
            TableRow::new(vec![
                Span::styled(format!("{:<7}", p.pid), Style::default().fg(ui.theme.fg)),
                Span::styled(
                    p_name,
                    Style::default()
                        .fg(ui.theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(primary, Style::default().fg(ui.theme.accent)),
                Span::styled(secondary, Style::default().fg(ui.theme.fg)),
                Span::styled(p.cmd.clone(), Style::default().fg(ui.theme.muted)),
            ])
        })
        .collect();

    if rows.is_empty() {
        let empty_msg = if state.metric == HistoryMetric::Gpu {
            "No active GPU processes recorded for this sample"
        } else {
            "No process activity recorded for this sample"
        };
        let p = Paragraph::new(empty_msg).style(Style::default().fg(ui.theme.muted));
        frame.render_widget(p, inner);
        return;
    }

    let widths = [
        Constraint::Length(8),
        Constraint::Length(18),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Min(0),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, inner);
}

fn format_proc_net_io(bps: u64, bytes: u64) -> String {
    if bps > 0 {
        format!("{}/s", cpu::human_bytes(bps))
    } else if bytes > 0 {
        cpu::human_bytes(bytes)
    } else {
        "--".to_string()
    }
}

fn format_proc_conns(proc: &HistoryProcess) -> String {
    let tot_bytes = proc.net_rx_bytes + proc.net_tx_bytes;
    let tot_str = if tot_bytes > 0 {
        format!("Tot {}", cpu::human_bytes(tot_bytes))
    } else {
        String::new()
    };
    let mut conns = Vec::new();
    if proc.tcp_est > 0 {
        conns.push(format!("{} est", proc.tcp_est));
    }
    if proc.udp > 0 {
        conns.push(format!("{} udp", proc.udp));
    }
    let conn_str = conns.join(", ");
    if !tot_str.is_empty() && !conn_str.is_empty() {
        format!("{} │ {}", tot_str, conn_str)
    } else if !tot_str.is_empty() {
        tot_str
    } else if !conn_str.is_empty() {
        conn_str
    } else {
        "--".to_string()
    }
}

fn draw_sessions_modal(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };

    let w = 82u16.min(area.width.saturating_sub(4));
    let h = 16u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, modal_area);

    let title = format!(" SAVED SESSIONS ({}) ", state.saved_recordings.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(title);
    frame.render_widget(block.clone(), modal_area);
    let inner = block.inner(modal_area);

    if inner.height < 4 || inner.width < 20 {
        return;
    }

    let [table_area, help_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let header = TableRow::new(vec![
        Span::styled("ID / FILENAME", Style::default().fg(ui.theme.muted)),
        Span::styled("DATE", Style::default().fg(ui.theme.muted)),
        Span::styled("TIME", Style::default().fg(ui.theme.muted)),
        Span::styled("DUR", Style::default().fg(ui.theme.muted)),
        Span::styled("FOCUS", Style::default().fg(ui.theme.muted)),
        Span::styled("SAMPLES", Style::default().fg(ui.theme.muted)),
    ]);

    let rows: Vec<TableRow> = state
        .saved_recordings
        .iter()
        .enumerate()
        .map(|(idx, rec)| {
            let is_selected = idx == state.selected_session_idx;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui.theme.fg)
            };
            let arrow = if is_selected { "▶ " } else { "  " };
            let id_str = format!("{}{}", arrow, rec.id);
            TableRow::new(vec![
                Span::styled(id_str, style),
                Span::styled(&rec.date, style),
                Span::styled(&rec.time, style),
                Span::styled(&rec.duration, style),
                Span::styled(&rec.metric_focus, style),
                Span::styled(format!("{}", rec.sample_count), style),
            ])
        })
        .collect();

    if rows.is_empty() {
        let empty_msg = Paragraph::new(
            "No saved sessions found in ~/.local/share/perfo/recordings\nPress 'r' in history view to record a session.",
        )
        .style(Style::default().fg(ui.theme.muted));
        frame.render_widget(empty_msg, table_area);
    } else {
        let widths = [
            Constraint::Length(26),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(0),
        ];
        let table = Table::new(rows, widths).header(header);
        frame.render_widget(table, table_area);
    }

    let help_line = Line::from(vec![
        Span::styled("↑/↓/j/k", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Select  "),
        Span::styled("Enter", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Load/Replay  "),
        Span::styled("d", Style::default().fg(ui.theme.red)),
        Span::raw(" Delete  "),
        Span::styled("Esc/s", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Close"),
    ]);
    frame.render_widget(Paragraph::new(help_line), help_area);
}

fn draw_record_modal(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };

    let w = 58u16.min(area.width.saturating_sub(4));
    let h = 13u16.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, modal_area);

    let title = " RECORDING SUBSYSTEMS ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(title);
    frame.render_widget(block.clone(), modal_area);
    let inner = block.inner(modal_area);

    if inner.height < 6 || inner.width < 24 {
        return;
    }

    let items = [
        (
            0,
            "1",
            "CPU",
            "Usage, Cores, Threads, CPU Procs",
            state.recording_mask.cpu,
        ),
        (
            1,
            "2",
            "Memory",
            "RAM, Swap, Process Memory",
            state.recording_mask.mem,
        ),
        (
            2,
            "3",
            "Disk I/O",
            "Read/Write Rates, IO Procs",
            state.recording_mask.io,
        ),
        (
            3,
            "4",
            "Network",
            "Bandwidth, Sockets, Net Procs",
            state.recording_mask.net,
        ),
        (
            4,
            "5",
            "GPU",
            "Usage, VRAM, GPU Procs",
            state.recording_mask.gpu,
        ),
        (
            5,
            "6",
            "NPU",
            "Neural Acceleration Engine",
            state.recording_mask.npu,
        ),
    ];

    let mut lines = Vec::new();
    for (idx, num, name, desc, checked) in items {
        let is_selected = state.record_modal_idx == idx;
        let prefix = if is_selected { "> " } else { "  " };
        let box_str = if checked { "[x] " } else { "[ ] " };
        let box_style = if checked {
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui.theme.muted)
        };
        let label_style = if is_selected {
            Style::default()
                .fg(ui.theme.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui.theme.fg)
        };
        let desc_style = Style::default().fg(ui.theme.muted);

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                if is_selected {
                    Style::default().fg(ui.theme.accent)
                } else {
                    Style::default()
                },
            ),
            Span::styled(box_str, box_style),
            Span::styled(format!("{}. {:<8}", num, name), label_style),
            Span::styled(format!(" {}", desc), desc_style),
        ]));
    }

    lines.push(Line::from(""));

    let btn_start_selected = state.record_modal_idx == 6;
    let btn_cancel_selected = state.record_modal_idx == 7;

    let start_style = if btn_start_selected {
        Style::default()
            .bg(ui.theme.accent)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(ui.theme.accent)
            .add_modifier(Modifier::BOLD)
    };
    let cancel_style = if btn_cancel_selected {
        Style::default()
            .bg(ui.theme.muted)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ui.theme.muted)
    };

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("[ Start Recording (r / Enter) ]", start_style),
        Span::raw("   "),
        Span::styled("[ Cancel (Esc) ]", cancel_style),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Space/Enter: Toggle  1-6: Direct Toggle  Esc: Cancel",
        Style::default().fg(ui.theme.muted),
    )]));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn clean_process_name(name: &str, cmd: &str, pid: u32) -> String {
    let raw = if !name.is_empty() {
        name
    } else {
        cmd.split_whitespace().next().unwrap_or("")
    };
    let trimmed = raw.trim_matches(&['"', '\''][..]);
    let base = match trimmed.rfind('/') {
        Some(idx) => &trimmed[idx + 1..],
        None => trimmed,
    };
    if base.is_empty() {
        pid.to_string()
    } else {
        base.to_string()
    }
}

fn format_local_time(time: std::time::SystemTime) -> String {
    let dur = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let sec = dur.as_secs() as libc::time_t;
    unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&sec, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn format_local_datetime(time: std::time::SystemTime) -> String {
    let dur = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let sec = dur.as_secs() as libc::time_t;
    unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&sec, &mut tm);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_cycling() {
        let mut m = HistoryMetric::Cpu;
        m = m.next();
        assert_eq!(m, HistoryMetric::Mem);
        m = m.next();
        assert_eq!(m, HistoryMetric::Io);
        m = m.next();
        assert_eq!(m, HistoryMetric::Net);
        m = m.next();
        assert_eq!(m, HistoryMetric::Gpu);
        m = m.next();
        assert_eq!(m, HistoryMetric::Cpu);
    }

    #[test]
    fn span_cycling_and_seconds() {
        let mut sp = HistorySpan::Span2m;
        assert_eq!(sp.seconds(500), 120);
        sp = sp.next();
        assert_eq!(sp, HistorySpan::Span15m);
        assert_eq!(sp.seconds(500), 900);
        sp = sp.next();
        assert_eq!(sp, HistorySpan::Span1h);
        assert_eq!(sp.seconds(500), 3600);
        sp = sp.next();
        assert_eq!(sp, HistorySpan::SpanAll);
        assert_eq!(sp.seconds(500), 500);
    }

    #[test]
    fn history_stepping_and_jumping() {
        let mut state = HistoryState::default();
        assert!(state.is_live());

        for i in 0..100 {
            state.samples.push_back(HistorySample {
                timestamp: format!("12:00:{:02}", i % 60),
                cpu: 10.0 + (i as f32 % 50.0),
                mem: 40.0,
                io_mb: 1.0,
                read_bps: 100,
                write_bps: 200,
                gpu: 0.0,
                net_rx_bps: 1000,
                net_tx_bps: 2000,
                top_procs: vec![],
            });
        }

        assert_eq!(state.effective_index(), 99);
        assert!(state.is_live());

        state.step(-1);
        assert_eq!(state.effective_index(), 98);
        assert!(!state.is_live());

        state.jump(-1);
        assert_eq!(state.effective_index(), 88);

        state.step(1);
        assert_eq!(state.effective_index(), 89);

        state.jump_to_live();
        assert_eq!(state.effective_index(), 99);
        assert!(state.is_live());
    }

    #[test]
    fn clean_process_name_removes_path_and_quotes() {
        assert_eq!(clean_process_name("/usr/bin/bash", "", 1234), "bash");
        assert_eq!(clean_process_name("\"rustc\"", "", 5678), "rustc");
        assert_eq!(
            clean_process_name("", "/usr/local/bin/python3 main.py", 999),
            "python3"
        );
        assert_eq!(clean_process_name("", "", 42), "42");
    }

    #[test]
    fn record_modal_navigation_and_toggle() {
        let mut state = HistoryState::default();
        assert!(!state.record_modal);
        assert_eq!(state.record_modal_idx, 0);

        state.open_record_modal();
        assert!(state.record_modal);

        state.record_modal_next();
        assert_eq!(state.record_modal_idx, 1);
        state.record_modal_prev();
        assert_eq!(state.record_modal_idx, 0);
        state.record_modal_prev();
        assert_eq!(state.record_modal_idx, 7);
        state.record_modal_next();
        assert_eq!(state.record_modal_idx, 0);

        // Toggle CPU (idx 0) off
        assert!(state.recording_mask.cpu);
        state.toggle_record_mask_item(0);
        assert!(!state.recording_mask.cpu);

        // Toggle CPU back on
        state.toggle_record_mask_item(0);
        assert!(state.recording_mask.cpu);

        // Guard: Cannot uncheck the last remaining item
        state.recording_mask = RecordingMask {
            cpu: true,
            mem: false,
            io: false,
            net: false,
            gpu: false,
            npu: false,
        };
        state.toggle_record_mask_item(0);
        assert!(state.recording_mask.cpu); // Remains true

        state.close_record_modal();
        assert!(!state.record_modal);
    }

    #[test]
    fn selective_recording_masks_samples() {
        let mut state = HistoryState {
            recording_mask: RecordingMask {
                cpu: true,
                mem: false,
                io: false,
                net: false,
                gpu: false,
                npu: false,
            },
            ..Default::default()
        };
        state.start_session_recording();
        assert!(state.is_session_recording);

        let mut monitor = crate::data::cpu::CpuMonitor::new();
        let snap = monitor.snapshot();
        state.record_snapshot(&snap);

        assert_eq!(state.session_record_buffer.len(), 1);
        let rec = &state.session_record_buffer[0];
        assert_eq!(rec.mem, 0.0);
        assert_eq!(rec.io_mb, 0.0);
        assert_eq!(rec.read_bps, 0);
        assert_eq!(rec.write_bps, 0);
        assert_eq!(rec.gpu, 0.0);
        assert_eq!(rec.net_rx_bps, 0);
        assert_eq!(rec.net_tx_bps, 0);
    }
}

//! The history page: the flight recorder's live and replay views.

mod chart;
mod draw;
mod export;
mod format;
mod modals;
mod nav;
mod record;
mod session;
mod tables;

pub(crate) use draw::draw_history;

use std::collections::VecDeque;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::data::cpu::RecordingMask;

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

#[cfg(test)]
mod tests {
    use super::format::clean_process_name;
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

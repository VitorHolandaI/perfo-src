//! The history page: the flight recorder's live and replay views.

mod chart;
mod draw;
pub mod duration;
mod export;
mod format;
mod modals;
mod nav;
mod record;
pub mod session;
mod tables;
pub mod traceable;

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
    /// The process's start time (epoch seconds) as of the recorded tick.
    ///
    /// A pid on its own does not identify a process across time, so a replay
    /// cannot tell whether pid 8231 from 40 minutes ago is the pid 8231 alive
    /// now. Defaulted so recordings written before this field still load; 0
    /// there means "unknown", not "started at the epoch".
    #[serde(default)]
    pub start_time: u64,
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
    /// Samples the running recording stops at, fixed when it starts from
    /// `record_duration` so changing the picker mid-recording does not move it.
    pub target_record_seconds: usize,
    pub record_duration: duration::RecordDuration,
    /// Digits typed for a custom duration, in minutes.
    pub record_custom_minutes: String,
    pub recording_mask: RecordingMask,
    pub record_modal: bool,
    pub record_modal_idx: usize,
    /// How many processes each recorded sample carries. The process table is
    /// ~99% of a sample's bytes (measured: 905 B/process against a 139 B
    /// remainder), so this is the one knob that decides a recording's size.
    pub record_process_depth: usize,

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
            record_duration: duration::RecordDuration::default(),
            record_custom_minutes: String::new(),
            recording_mask: RecordingMask::ALL,
            record_modal: false,
            record_modal_idx: 0,
            record_process_depth: session::default_process_depth(),
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
        assert_eq!(state.record_modal_idx, session::CANCEL_BUTTON_IDX);
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

    /// The depth stepper wraps in both directions and lands on every preset.
    #[test]
    fn process_depth_cycles_through_every_preset() {
        let mut state = HistoryState {
            record_process_depth: session::PROCESS_DEPTHS[0],
            ..HistoryState::default()
        };

        let mut seen = Vec::new();
        for _ in 0..session::PROCESS_DEPTHS.len() {
            seen.push(state.record_process_depth);
            state.cycle_process_depth(true);
        }
        assert_eq!(seen, session::PROCESS_DEPTHS.to_vec());
        // One more step past the end wraps to the first preset.
        assert_eq!(state.record_process_depth, session::PROCESS_DEPTHS[0]);

        state.cycle_process_depth(false);
        assert_eq!(
            state.record_process_depth,
            *session::PROCESS_DEPTHS.last().unwrap()
        );
    }

    /// A depth set through PERFO_RECORD_DEPTH need not be one of the presets;
    /// stepping forward must not throw it away by wrapping to the first.
    #[test]
    fn an_off_preset_depth_enters_the_list_at_the_next_preset_above_it() {
        let mut state = HistoryState {
            record_process_depth: 42,
            ..HistoryState::default()
        };
        state.cycle_process_depth(true);
        assert_eq!(state.record_process_depth, 50);

        // Above every finite preset, the only thing left is "all".
        state.record_process_depth = 500;
        state.cycle_process_depth(true);
        assert_eq!(state.record_process_depth, session::PROCESS_DEPTHS[0]);
    }

    /// 0 is "every process", not "no processes".
    #[test]
    fn depth_zero_reads_as_all() {
        assert_eq!(session::process_depth_label(0), "all");
        assert_eq!(session::process_depth_label(30), "30");
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

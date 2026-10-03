//! Whether a process seen in a replayed sample is still the same process.
//!
//! A recording holds counters, not syscalls, so there is nothing in it to
//! feed a tracer: `perfo trace` attaches to a live process with PTRACE_SEIZE
//! (src/trace/mod.rs:344). The most a replay can offer is "this pid is still
//! the process you were looking at, go trace it now" -- and a pid alone
//! cannot say that, because the kernel reuses pid numbers.

use super::HistoryProcess;
use crate::data::cpu::ProcessInfo;

/// What a replayed row's pid means on the live system right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceTarget {
    /// Same pid, same start time: the process the recording saw is running.
    Alive,
    /// Nothing holds that pid any more.
    Exited,
    /// Something holds that pid, but it is not what was recorded. Tracing it
    /// would attach to an unrelated process.
    Reused,
    /// Identity cannot be established: the recording predates the start_time
    /// field, or the live process list is not being collected.
    Unknown,
}

impl TraceTarget {
    /// A one-character column marker, chosen to read without colour.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Alive => "*",
            Self::Exited => "-",
            Self::Reused => "!",
            Self::Unknown => "?",
        }
    }

    /// What the footer says about the selected row.
    pub fn explain(self) -> &'static str {
        match self {
            Self::Alive => "still running, same process: perfo trace <pid> works",
            Self::Exited => "exited since the recording",
            Self::Reused => "pid reused by another process, do not trace it",
            Self::Unknown => "recorded before start times were saved",
        }
    }
}

/// Matches a recorded process against the live process list.
///
/// `live` is the current snapshot's table, so both sides come from the same
/// source in the same units and there is no clock skew to reason about.
pub fn trace_target(recorded: &HistoryProcess, live: &[ProcessInfo]) -> TraceTarget {
    // No live table means the pane is not collecting processes; absence of
    // evidence is not evidence the process exited.
    if live.is_empty() || recorded.start_time == 0 {
        return TraceTarget::Unknown;
    }
    match live.iter().find(|p| p.pid == recorded.pid) {
        None => TraceTarget::Exited,
        Some(p) if p.start_time == recorded.start_time => TraceTarget::Alive,
        Some(_) => TraceTarget::Reused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded(pid: u32, start_time: u64) -> HistoryProcess {
        HistoryProcess {
            pid,
            start_time,
            ..HistoryProcess::default()
        }
    }

    fn live(pid: u32, start_time: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            start_time,
            ..ProcessInfo::default()
        }
    }

    #[test]
    fn same_pid_and_start_time_is_the_same_process() {
        let table = [live(8231, 1789158164)];
        assert_eq!(
            trace_target(&recorded(8231, 1789158164), &table),
            TraceTarget::Alive
        );
    }

    #[test]
    fn a_pid_missing_from_the_live_table_has_exited() {
        let table = [live(9000, 1789158164)];
        assert_eq!(
            trace_target(&recorded(8231, 1789158164), &table),
            TraceTarget::Exited
        );
    }

    /// The case the whole field exists for: the number is back, the process
    /// is not.
    #[test]
    fn the_same_pid_with_a_later_start_time_was_reused() {
        let table = [live(8231, 1789208047)];
        assert_eq!(
            trace_target(&recorded(8231, 1789158164), &table),
            TraceTarget::Reused
        );
    }

    /// Recordings written before this field default start_time to 0, which
    /// must not be read as "started at the epoch" and reported as reused.
    #[test]
    fn a_recording_without_start_times_is_unknown_not_reused() {
        let table = [live(8231, 1789208047)];
        assert_eq!(
            trace_target(&recorded(8231, 0), &table),
            TraceTarget::Unknown
        );
    }

    /// The recordings already on disk have no start_time field at all, so
    /// loading one must not fail -- it must land on Unknown.
    #[test]
    fn a_sample_written_before_this_field_still_deserializes() {
        let json = r#"{"pid":8231,"name":"claude","cmd":"claude","cpu_percent":1.0,
            "mem_bytes":100,"read_bps":0,"write_bps":0,"gpu_percent":0.0,
            "vram_bytes":0,"net_rx_bps":0,"net_tx_bps":0,"net_rx_bytes":0,
            "net_tx_bytes":0,"tcp_est":0,"udp":0}"#;
        let p: HistoryProcess = serde_json::from_str(json).expect("old sample must load");
        assert_eq!(p.pid, 8231);
        assert_eq!(p.start_time, 0);
        assert_eq!(
            trace_target(&p, &[live(8231, 1789208047)]),
            TraceTarget::Unknown
        );
    }

    /// A pane that is not collecting processes must not make every row look
    /// like it exited.
    #[test]
    fn an_empty_live_table_is_unknown_not_exited() {
        assert_eq!(
            trace_target(&recorded(8231, 1789158164), &[]),
            TraceTarget::Unknown
        );
    }
}

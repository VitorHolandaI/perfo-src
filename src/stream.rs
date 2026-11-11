//! The `perfo stream --json` collector: the long-running process the
//! Omarchy widget drives over a pipe.
//!
//! It is the only command that outlives a single answer, so it is the only
//! one that has to notice its client going away. Two things end it: the
//! control pipe on stdin closing, and a write failing because nobody reads
//! the other end. Neither is a failure.

use crate::data;
use crate::data::cpu::{CollectionPlan, CollectionProfile};
use std::process::ExitCode;

/// Whether a file of this kind is a channel a client holds open. A pipe or a
/// socket is; a terminal, a regular file and /dev/null are not, so EOF on one
/// of those is no evidence that anybody went away.
fn mode_is_client_channel(mode: libc::mode_t) -> bool {
    let kind = mode & libc::S_IFMT;
    kind == libc::S_IFIFO || kind == libc::S_IFSOCK
}

/// Whether stdin is a channel a client holds open.
fn stdin_is_client_channel() -> bool {
    // SAFETY: fstat writes only into the stat handed to it, and fd 0 exists in
    // any process the kernel started normally.
    let info = unsafe {
        let mut info = std::mem::MaybeUninit::<libc::stat>::zeroed();
        if libc::fstat(libc::STDIN_FILENO, info.as_mut_ptr()) != 0 {
            return false;
        }
        info.assume_init()
    };
    mode_is_client_channel(info.st_mode)
}

/// What draining the control channel found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControlState {
    /// The client still holds its end, so more lines may arrive.
    Open,
    /// Its end is closed: nothing will ever arrive again.
    Closed,
}

/// Applies every control line waiting, and reports whether the channel
/// survived. Draining to empty each tick keeps a burst of control lines from
/// taking one sample interval each.
fn drain_control(
    control: &std::sync::mpsc::Receiver<String>,
    monitor: &mut JsonStreamMonitor,
) -> ControlState {
    use std::sync::mpsc::TryRecvError;
    loop {
        match control.try_recv() {
            Ok(line) => monitor.apply_control(&line),
            Err(TryRecvError::Empty) => return ControlState::Open,
            Err(TryRecvError::Disconnected) => return ControlState::Closed,
        }
    }
}

/// Writes one sample line and pushes it out. An unflushed stream leaves the
/// widget showing stale numbers.
fn write_sample(out: &mut impl std::io::Write, json: &str) -> std::io::Result<()> {
    writeln!(out, "{json}")?;
    out.flush()
}

/// Emits one snapshot per sample interval until a refresh or a write fails.
///
/// Control lines arriving on stdin narrow what each refresh collects, so a
/// client showing one pane does not pay for the panes it is hiding.
// qual:allow(test_quality, untested) reason: "the streaming loop itself; apply_control and refresh_json, which it drives, are tested"
pub(crate) fn run_stream(summary: bool) -> ExitCode {
    let mut monitor = JsonStreamMonitor::new(summary);
    let control = spawn_stream_control();
    // A client holding stdin open is how this process learns it is still
    // wanted. Its own writes cannot tell it: an orphan whose output goes
    // somewhere that never fails, such as /dev/null, used to keep sampling
    // once a second forever.
    let client_holds_stdin = stdin_is_client_channel();
    let mut out = std::io::stdout();
    loop {
        data::cpu::wait_sample_interval();
        if drain_control(&control, &mut monitor) == ControlState::Closed && client_holds_stdin {
            return ExitCode::SUCCESS;
        }
        let json = match monitor.refresh_json() {
            Ok(json) => json,
            Err(e) => {
                eprintln!("perfo stream: {e}");
                return ExitCode::FAILURE;
            }
        };
        match write_sample(&mut out, &json) {
            Ok(()) => {}
            // What `| head -1` leaves behind. Ending a stream tool that way is
            // ordinary, so it must not read as a crash; `println!` panics here.
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perfo stream: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
}

/// Reads control lines from stdin without blocking the sampling loop.
///
/// A client that is showing one pane can say so, and the collector then stops
/// gathering what that pane does not display. The protocol is one command per
/// line: `profile <name>`, `recording on`, `recording off`.
// qual:allow(test_quality, untested) reason: "reads the process's real stdin on a thread; apply_control covers the protocol it feeds"
fn spawn_stream_control() -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if tx.send(line.trim().to_string()).is_err() {
                        return;
                    }
                }
            }
        }
    });
    rx
}

/// Presentation choices a client can make about the stream it receives.
#[derive(Clone, Copy)]
struct StreamOptions {
    /// List threads as their own rows. A client with no way to tell a thread
    /// from a process should leave this off.
    threads: bool,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self { threads: true }
    }
}

enum JsonStreamMonitor {
    Full(Box<data::cpu::CpuMonitor>, CollectionPlan, StreamOptions),
    Summary(Box<data::summary::WidgetSummaryMonitor>),
}

impl JsonStreamMonitor {
    fn new(summary: bool) -> Self {
        if summary {
            Self::Summary(Box::default())
        } else {
            Self::Full(
                Box::default(),
                CollectionPlan::default(),
                StreamOptions::default(),
            )
        }
    }

    /// Narrows collection to what the client says it is showing. Unknown names
    /// are ignored so an older widget keeps working.
    fn set_profile(&mut self, name: &str) {
        if let Self::Full(_, plan, _) = self {
            if let Ok(profile) = name.parse::<CollectionProfile>() {
                plan.visible = profile;
            }
        }
    }

    /// Whether individual threads appear alongside their process.
    fn set_threads(&mut self, threads: bool) {
        if let Self::Full(_, _, options) = self {
            options.threads = threads;
        }
    }

    /// Narrows a recording to the subsystems the user ticked in the picker, so
    /// recording only CPU does not also walk every process's sockets.
    fn set_recording_mask(&mut self, subsystems: &str) {
        if let Self::Full(_, plan, _) = self {
            // Default is ALL, so start from nothing and turn on what the
            // client listed.
            let mut mask = data::cpu::RecordingMask::NONE;
            for name in subsystems.split(',') {
                match name.trim() {
                    "cpu" => mask.cpu = true,
                    "mem" => mask.mem = true,
                    "io" => mask.io = true,
                    "net" => mask.net = true,
                    "gpu" => mask.gpu = true,
                    "npu" => mask.npu = true,
                    "temp" => mask.temp = true,
                    _ => {}
                }
            }
            if !mask.is_empty() {
                plan.recording_mask = mask;
            }
        }
    }

    /// Turns background recording on or off, which unions the history needs
    /// into whatever the visible pane already asks for.
    fn set_recording(&mut self, recording: bool) {
        if let Self::Full(_, plan, _) = self {
            plan.recording = recording;
        }
    }

    /// Applies one control line. Unknown commands are ignored so a newer
    /// widget cannot break an older collector, or the other way round.
    fn apply_control(&mut self, line: &str) {
        match line.split_once(' ') {
            Some(("profile", name)) => self.set_profile(name),
            Some(("recording", "on")) => self.set_recording(true),
            Some(("recording", "off")) => self.set_recording(false),
            Some(("mask", subsystems)) => self.set_recording_mask(subsystems),
            Some(("threads", "on")) => self.set_threads(true),
            Some(("threads", "off")) => self.set_threads(false),
            _ => {}
        }
    }

    fn refresh_json(&mut self) -> serde_json::Result<String> {
        match self {
            Self::Full(monitor, plan, options) => {
                monitor.refresh_for(*plan, true);
                let mut snapshot = monitor.snapshot_for(*plan);
                if !options.threads {
                    // A thread's CPU time is already counted in its process, so
                    // listing both double-counts the column and fills the table
                    // with rows that are parts of entries already shown.
                    snapshot.processes.retain(|p| p.owner.is_none());
                }
                serde_json::to_string(&snapshot)
            }
            Self::Summary(monitor) => {
                monitor.refresh();
                serde_json::to_string(&monitor.snapshot())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a pipe or a socket is a client's end. Classifying a terminal or
    /// /dev/null as one would end any stream started with `< /dev/null` on its
    /// first tick.
    #[test]
    fn only_a_pipe_or_socket_counts_as_a_client_channel() {
        assert!(mode_is_client_channel(libc::S_IFIFO | 0o600));
        assert!(mode_is_client_channel(libc::S_IFSOCK | 0o600));
        assert!(!mode_is_client_channel(libc::S_IFCHR | 0o600));
        assert!(!mode_is_client_channel(libc::S_IFREG | 0o644));
        assert!(!mode_is_client_channel(libc::S_IFDIR | 0o755));
    }

    /// A burst of control lines has to be applied within one tick, not one
    /// line per sample interval.
    #[test]
    fn draining_applies_every_pending_line_at_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("profile cpu".to_string()).unwrap();
        tx.send("threads off".to_string()).unwrap();
        let mut monitor = stream_monitor();
        assert_eq!(drain_control(&rx, &mut monitor), ControlState::Open);
        let JsonStreamMonitor::Full(_, plan, options) = &monitor else {
            panic!("expected the full monitor");
        };
        assert_eq!(plan.visible, CollectionProfile::Cpu);
        assert!(!options.threads);
    }

    /// The sender dropping is the client closing its end, and the loop has to
    /// see that rather than treat it as "nothing waiting".
    #[test]
    fn draining_reports_a_closed_channel() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        drop(tx);
        let mut monitor = stream_monitor();
        assert_eq!(drain_control(&rx, &mut monitor), ControlState::Closed);
    }

    /// The monitor a widget client gets, so the control protocol can be driven
    /// without a live stdin. `new(false)` is the construction `run` uses.
    fn stream_monitor() -> JsonStreamMonitor {
        JsonStreamMonitor::new(false)
    }

    fn plan_of(monitor: &JsonStreamMonitor) -> CollectionPlan {
        match monitor {
            JsonStreamMonitor::Full(_, plan, _) => *plan,
            JsonStreamMonitor::Summary(_) => panic!("expected a Full stream monitor"),
        }
    }

    fn threads_of(monitor: &JsonStreamMonitor) -> bool {
        match monitor {
            JsonStreamMonitor::Full(_, _, options) => options.threads,
            JsonStreamMonitor::Summary(_) => panic!("expected a Full stream monitor"),
        }
    }

    #[test]
    fn profile_control_switches_the_visible_pane() {
        let mut monitor = stream_monitor();

        monitor.apply_control("profile cpu");
        assert_eq!(plan_of(&monitor).visible, CollectionProfile::Cpu);

        // The closed panel still streams, so "hidden" has to be reachable.
        monitor.apply_control("profile hidden");
        assert_eq!(plan_of(&monitor).visible, CollectionProfile::Hidden);
    }

    #[test]
    fn unknown_profile_leaves_the_previous_one_in_place() {
        let mut monitor = stream_monitor();
        monitor.apply_control("profile mem");

        // A newer widget may name a pane this collector does not have; it must
        // keep serving the last pane it understood rather than reset.
        monitor.apply_control("profile quantum");
        assert_eq!(plan_of(&monitor).visible, CollectionProfile::Mem);
    }

    #[test]
    fn mask_enables_only_the_subsystems_the_client_listed() {
        let mut monitor = stream_monitor();
        assert_eq!(
            plan_of(&monitor).recording_mask,
            data::cpu::RecordingMask::ALL
        );

        monitor.apply_control("mask cpu,net");
        let mask = plan_of(&monitor).recording_mask;
        assert!(mask.cpu && mask.net);
        assert!(!mask.mem && !mask.io && !mask.gpu && !mask.npu && !mask.temp);

        // Every flag the mask carries needs a name the client can send, or the
        // picker can tick something the stream has no way to hear about.
        monitor.apply_control("mask temp");
        let thermal_only = plan_of(&monitor).recording_mask;
        assert!(thermal_only.temp);
        assert_eq!(thermal_only.count(), 1);
    }

    #[test]
    fn mask_naming_nothing_known_is_ignored() {
        let mut monitor = stream_monitor();
        monitor.apply_control("mask cpu");

        // An all-off mask would silently record nothing, so it is refused and
        // the last usable mask survives.
        monitor.apply_control("mask nonsense");
        assert_eq!(plan_of(&monitor).recording_mask.count(), 1);
        assert!(plan_of(&monitor).recording_mask.cpu);
    }

    #[test]
    fn recording_and_threads_toggle_both_ways() {
        let mut monitor = stream_monitor();

        monitor.apply_control("recording on");
        assert!(plan_of(&monitor).recording);
        monitor.apply_control("recording off");
        assert!(!plan_of(&monitor).recording);

        monitor.apply_control("threads off");
        assert!(!threads_of(&monitor));
        monitor.apply_control("threads on");
        assert!(threads_of(&monitor));
    }

    #[test]
    fn unknown_control_lines_are_ignored() {
        let mut monitor = stream_monitor();
        let before = plan_of(&monitor);

        for line in ["", "profile", "recording maybe", "threads", "garbage line"] {
            monitor.apply_control(line);
        }
        assert_eq!(plan_of(&monitor), before);
        assert!(threads_of(&monitor));
    }

    /// Regression: thread rows made the widget's CPU column read ~300%, because
    /// a thread's time is already counted in its process.
    #[test]
    fn threads_off_drops_rows_that_are_parts_of_another_row() {
        let mut monitor = stream_monitor();
        monitor.apply_control("profile cpu");
        monitor.apply_control("threads off");

        let json = monitor.refresh_json().expect("snapshot serializes");
        let snapshot: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let processes = snapshot["processes"]
            .as_array()
            .expect("snapshot carries a process list");

        assert!(
            processes.iter().all(|p| p["owner"].is_null()),
            "threads off must leave only owning processes"
        );
    }
}

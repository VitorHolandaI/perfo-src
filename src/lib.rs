//! perfo: system performance monitor.
//!
//! The library crate holds all logic so integration tests (in `tests/`)
//! can exercise the public API; `main.rs` is a thin wrapper around
//! [`run`]. The TUI, data collectors (CPU/memory/disk/network), and the
//! ptrace tracer live here.

pub mod data;
pub mod recordings;
pub mod theme;
pub mod trace;
pub mod tui;
pub mod units;

use data::cpu::{CollectionPlan, CollectionProfile};

use std::process::ExitCode;

enum Command {
    Help,
    Version,
    Tui,
    TuiHistory,
    CpuJson,
    StreamJson {
        summary: bool,
    },
    Record {
        subcmd: String,
        args: Vec<String>,
    },
    Bench {
        secs: u64,
    },
    Trace {
        pid: i32,
        filter: Option<String>,
        cmd: Option<Vec<String>>,
    },
    Export {
        basename: String,
    },
}

fn parse(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        None | Some("tui") => Command::Tui,
        Some("hist") | Some("history") | Some("--history") => Command::TuiHistory,
        Some("-h") | Some("--help") | Some("help") => Command::Help,
        Some("-V") | Some("--version") | Some("version") => Command::Version,
        Some("cpu") => Command::CpuJson,
        Some("stream") => Command::StreamJson {
            summary: args.iter().skip(1).any(|arg| arg == "--summary"),
        },
        Some("record") | Some("records") | Some("recordings") => {
            let subcmd = args.get(1).cloned().unwrap_or_else(|| "list".to_string());
            let subargs: Vec<String> = args.iter().skip(2).cloned().collect();
            Command::Record {
                subcmd,
                args: subargs,
            }
        }
        Some("export") => {
            let basename = args.get(1).cloned().unwrap_or_else(|| {
                let now = std::time::SystemTime::now();
                let dur = now
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                let sec = dur.as_secs() as libc::time_t;
                unsafe {
                    let mut tm = std::mem::zeroed::<libc::tm>();
                    libc::localtime_r(&sec, &mut tm);
                    format!(
                        "perfo-history-{:04}{:02}{:02}-{:02}{:02}{:02}",
                        tm.tm_year + 1900,
                        tm.tm_mon + 1,
                        tm.tm_mday,
                        tm.tm_hour,
                        tm.tm_min,
                        tm.tm_sec
                    )
                }
            });
            Command::Export { basename }
        }
        Some("bench") => {
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(15);
            Command::Bench { secs }
        }
        Some("trace") => {
            if args.get(1).map(String::as_str) == Some("--") {
                let cmd: Vec<String> = args.iter().skip(2).cloned().collect();
                if cmd.is_empty() {
                    Command::Help
                } else {
                    Command::Trace {
                        pid: -1,
                        filter: None,
                        cmd: Some(cmd),
                    }
                }
            } else {
                let pid = args.get(1).and_then(|s| s.parse().ok());
                match pid {
                    Some(pid) => Command::Trace {
                        pid,
                        filter: args.get(2).cloned(),
                        cmd: None,
                    },
                    None => Command::Help,
                }
            }
        }
        _ => Command::Help,
    }
}

// qual:allow(test_quality, untested) reason: "writes a constant usage string to stdout"
fn print_help() {
    println!(
        "perfo {} - system performance monitor

USAGE:
  perfo                 interactive TUI (CPU focus)
  perfo hist | history  interactive history mode (timeline replay & export)
  perfo cpu --json      one-shot JSON snapshot (for widgets/scripts)
  perfo stream --json   continuous full JSON snapshots
  perfo stream --json --summary
                        continuous CPU/memory/GPU/NPU bar snapshots
  perfo record list     list saved session recordings (JSON)
  perfo record save <f> save session recording JSON file
  perfo record get <id> output recorded session JSON
  perfo record delete <id> delete recorded session
  perfo trace <pid> [name]
                        trace a process's syscalls (ptrace, no strace needed)
  perfo trace -- <cmd...>
                        spawn a command and trace its syscalls
  perfo bench [secs]    profile refresh/snapshot loop
  perfo -h | --help     show this help
  perfo -V | --version  show version
",
        env!("CARGO_PKG_VERSION")
    );
}

/// Maps a command's outcome onto an exit code, naming the subcommand in the
/// failure line so `perfo record: ...` and `perfo trace: ...` stay
/// distinguishable in a shell history.
fn report(context: &str, result: std::io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{context}: {e}");
            ExitCode::FAILURE
        }
    }
}

// qual:allow(test_quality, untested) reason: "prints one CpuMonitor snapshot; the monitor and its serialisation are tested"
fn run_cpu_json() -> ExitCode {
    let mut monitor = data::cpu::CpuMonitor::new();
    // A single sample carries no deltas, so the first interval is the price of
    // reporting a CPU percentage at all.
    data::cpu::wait_sample_interval();
    monitor.refresh();
    match serde_json::to_string_pretty(&monitor.snapshot()) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("perfo: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Emits one snapshot per sample interval until a refresh or a write fails.
///
/// Control lines arriving on stdin narrow what each refresh collects, so a
/// client showing one pane does not pay for the panes it is hiding.
// qual:allow(test_quality, untested) reason: "the streaming loop itself; apply_control and refresh_json, which it drives, are tested"
fn run_stream(summary: bool) -> ExitCode {
    let mut monitor = JsonStreamMonitor::new(summary);
    let control = spawn_stream_control();
    loop {
        data::cpu::wait_sample_interval();
        while let Ok(line) = control.try_recv() {
            monitor.apply_control(&line);
        }
        let json = match monitor.refresh_json() {
            Ok(json) => json,
            Err(e) => {
                eprintln!("perfo stream: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!("{json}");
        // An unflushed stream leaves the widget showing stale numbers, and a
        // closed pipe is how this loop learns the client is gone.
        if let Err(e) = std::io::Write::flush(&mut std::io::stdout()) {
            eprintln!("perfo stream: {e}");
            return ExitCode::FAILURE;
        }
    }
}

// qual:allow(test_quality, untested) reason: "prints the result of export_from_stdin, which consumes the process's real stdin"
fn run_export(basename: &str) -> ExitCode {
    match recordings::export_from_stdin(basename) {
        Ok((txt, json)) => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "exported",
                    "txt": txt,
                    "json": json,
                })
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("perfo export: {e}");
            ExitCode::FAILURE
        }
    }
}

// qual:allow(test_quality, untested) reason: "needs ptrace against a live process; the tracer loop is covered by the trace tests"
fn run_trace(pid: i32, filter: Option<&str>, cmd: Option<&[String]>) -> ExitCode {
    let result = match cmd {
        Some(argv) => trace::spawn(argv, filter),
        None => trace::attach(pid, filter),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("perfo trace: {e}");
            // Under the default yama ptrace_scope=1 this is a permission
            // refusal, not a bug, and the two ways out are worth spelling out.
            eprintln!(
                "hint: tracing an existing process only works for your own children \
                 (yama ptrace_scope=1); use `perfo trace -- <command>` to spawn it, \
                 or `sudo sysctl kernel.yama.ptrace_scope=0`"
            );
            ExitCode::FAILURE
        }
    }
}

// qual:allow(test_quality, untested) reason: "times CpuMonitor::refresh in a wall-clock loop; there is nothing to assert but the clock"
fn run_bench(secs: u64) -> ExitCode {
    let mut monitor = data::cpu::CpuMonitor::new();
    let start = std::time::Instant::now();
    let mut n = 0u32;
    while start.elapsed().as_secs() < secs {
        monitor.refresh();
        let _ = monitor.snapshot();
        n += 1;
    }
    let ms = start.elapsed().as_millis() as f64 / n.max(1) as f64;
    eprintln!("perfo bench: {n} full refreshes in {secs}s ({ms:.1} ms each)");
    ExitCode::SUCCESS
}

/// Parse args and dispatch the requested command.
// qual:allow(test_quality, untested) reason: "CLI entry point; parse() is tested and each command's logic has its own tests"
pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Command::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("perfo {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Tui => report("perfo", tui::run()),
        Command::TuiHistory => report("perfo", tui::run_with_pane(tui::cpu::Pane::History)),
        Command::CpuJson => run_cpu_json(),
        Command::StreamJson { summary } => run_stream(summary),
        Command::Record { subcmd, args } => {
            report("perfo record", recordings::dispatch(&subcmd, &args))
        }
        Command::Export { basename } => run_export(&basename),
        Command::Trace { pid, filter, cmd } => run_trace(pid, filter.as_deref(), cmd.as_deref()),
        Command::Bench { secs } => run_bench(secs),
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
            let mut mask = data::cpu::RecordingMask {
                cpu: false,
                mem: false,
                io: false,
                net: false,
                gpu: false,
                npu: false,
            };
            for name in subsystems.split(',') {
                match name.trim() {
                    "cpu" => mask.cpu = true,
                    "mem" => mask.mem = true,
                    "io" => mask.io = true,
                    "net" => mask.net = true,
                    "gpu" => mask.gpu = true,
                    "npu" => mask.npu = true,
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

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
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
        assert!(!mask.mem && !mask.io && !mask.gpu && !mask.npu);
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

    #[test]
    fn stream_summary_flag_selects_lightweight_monitor() {
        assert!(matches!(
            parse(&args(&["stream", "--json", "--summary"])),
            Command::StreamJson { summary: true }
        ));
        assert!(matches!(
            parse(&args(&["stream", "--json"])),
            Command::StreamJson { summary: false }
        ));
    }
}

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

/// Parse args and dispatch the requested command.
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
        Command::Tui => match tui::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perfo: {e}");
                ExitCode::FAILURE
            }
        },
        Command::TuiHistory => match tui::run_with_pane(tui::cpu::Pane::History) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perfo: {e}");
                ExitCode::FAILURE
            }
        },
        Command::CpuJson => {
            let mut monitor = data::cpu::CpuMonitor::new();
            data::cpu::wait_sample_interval();
            monitor.refresh();
            let snap = monitor.snapshot();
            match serde_json::to_string_pretty(&snap) {
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
        Command::StreamJson { summary } => {
            let mut monitor = JsonStreamMonitor::new(summary);
            let control = spawn_stream_control();
            loop {
                data::cpu::wait_sample_interval();
                while let Ok(line) = control.try_recv() {
                    monitor.apply_control(&line);
                }
                match monitor.refresh_json() {
                    Ok(json) => {
                        println!("{json}");
                        if let Err(e) = std::io::Write::flush(&mut std::io::stdout()) {
                            eprintln!("perfo stream: {e}");
                            return ExitCode::FAILURE;
                        }
                    }
                    Err(e) => {
                        eprintln!("perfo stream: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
        Command::Record { subcmd, args } => match recordings::dispatch(&subcmd, &args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("perfo record: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Export { basename } => match recordings::export_from_stdin(&basename) {
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
        },
        Command::Trace { pid, filter, cmd } => {
            let result = match cmd {
                Some(c) => trace::spawn(&c, filter.as_deref()),
                None => trace::attach(pid, filter.as_deref()),
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("perfo trace: {e}");
                    eprintln!(
                        "hint: tracing an existing process only works for your own children \
                         (yama ptrace_scope=1); use `perfo trace -- <command>` to spawn it, \
                         or `sudo sysctl kernel.yama.ptrace_scope=0`"
                    );
                    ExitCode::FAILURE
                }
            }
        }
        Command::Bench { secs } => {
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
    }
}

/// Reads control lines from stdin without blocking the sampling loop.
///
/// A client that is showing one pane can say so, and the collector then stops
/// gathering what that pane does not display. The protocol is one command per
/// line: `profile <name>`, `recording on`, `recording off`.
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

enum JsonStreamMonitor {
    Full(Box<data::cpu::CpuMonitor>, CollectionPlan),
    Summary(Box<data::summary::WidgetSummaryMonitor>),
}

impl JsonStreamMonitor {
    fn new(summary: bool) -> Self {
        if summary {
            Self::Summary(Box::default())
        } else {
            Self::Full(Box::default(), CollectionPlan::default())
        }
    }

    /// Narrows collection to what the client says it is showing. Unknown names
    /// are ignored so an older widget keeps working.
    fn set_profile(&mut self, name: &str) {
        if let Self::Full(_, plan) = self {
            if let Ok(profile) = name.parse::<CollectionProfile>() {
                plan.visible = profile;
            }
        }
    }

    /// Narrows a recording to the subsystems the user ticked in the picker, so
    /// recording only CPU does not also walk every process's sockets.
    fn set_recording_mask(&mut self, subsystems: &str) {
        if let Self::Full(_, plan) = self {
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
        if let Self::Full(_, plan) = self {
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
            _ => {}
        }
    }

    fn refresh_json(&mut self) -> serde_json::Result<String> {
        match self {
            Self::Full(monitor, plan) => {
                monitor.refresh_for(*plan, true);
                serde_json::to_string(&monitor.snapshot_for(*plan))
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

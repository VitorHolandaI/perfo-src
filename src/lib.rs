//! perfo: system performance monitor.
//!
//! The library crate holds all logic so integration tests (in `tests/`)
//! can exercise the public API; `main.rs` is a thin wrapper around
//! [`run`]. The TUI, data collectors (CPU/memory/disk/network), and the
//! ptrace tracer live here.

pub mod data;
pub mod theme;
pub mod trace;
pub mod tui;

use std::process::ExitCode;

enum Command {
    Help,
    Version,
    Tui,
    CpuJson,
    StreamJson,
    Bench {
        secs: u64,
    },
    Trace {
        pid: i32,
        filter: Option<String>,
        cmd: Option<Vec<String>>,
    },
}

fn parse(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        None | Some("tui") => Command::Tui,
        Some("-h") | Some("--help") | Some("help") => Command::Help,
        Some("-V") | Some("--version") | Some("version") => Command::Version,
        Some("cpu") => Command::CpuJson,
        Some("stream") => Command::StreamJson,
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
  perfo cpu --json      one-shot JSON snapshot (for widgets/scripts)
  perfo stream --json   continuous JSON snapshots (for widgets)
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
        Command::StreamJson => {
            let mut monitor = data::cpu::CpuMonitor::new();
            loop {
                data::cpu::wait_sample_interval();
                monitor.refresh();
                let snap = monitor.snapshot();
                match serde_json::to_string(&snap) {
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

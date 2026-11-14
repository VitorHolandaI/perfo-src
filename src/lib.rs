//! perfo: system performance monitor.
//!
//! The library crate holds all logic so integration tests (in `tests/`)
//! can exercise the public API; `main.rs` is a thin wrapper around
//! [`run`]. The TUI, data collectors (CPU/memory/disk/network), and the
//! ptrace tracer live here.

mod bounded_child;
pub mod data;
pub mod recordings;
mod stream;
pub mod theme;
pub mod trace;
pub mod tui;
pub mod units;

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
        Command::StreamJson { summary } => stream::run_stream(summary),
        Command::Record { subcmd, args } => {
            report("perfo record", recordings::dispatch(&subcmd, &args))
        }
        Command::Export { basename } => run_export(&basename),
        Command::Trace { pid, filter, cmd } => run_trace(pid, filter.as_deref(), cmd.as_deref()),
        Command::Bench { secs } => run_bench(secs),
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

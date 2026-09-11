//! The status line, and how it degrades as the terminal narrows.

use super::cpu::Pane;
use super::{Lang, State};

/// Terminal widths at which the status line can afford more key hints. The
/// numbers are the measured width of the line once that group is appended.
const WIDTH_FOR_CORE_SORT_KEYS: usize = 120;
const WIDTH_FOR_PROC_SORT_KEYS: usize = 135;
const WIDTH_FOR_TREE_AND_TRACE: usize = 160;
const WIDTH_FOR_THREAD_FILTERS: usize = 190;

const STATUS_SUFFIX: &str = " | ? help | q quit";

/// Modes that take over the whole status line: they replace the hints rather
/// than competing with them for space.
fn status_line_override(state: &State) -> Option<String> {
    if state.searching {
        return Some(format!("/{}{}", state.search, "_"));
    }
    if state.kill_prompt {
        if let Some(pid) = state.selected_pid {
            return Some(match state.lang {
                Lang::Pt => format!("matar {pid}?  1=SIGTERM  9=SIGKILL  0=cancela"),
                Lang::En => format!("kill {pid}?  1=SIGTERM  9=SIGKILL  0=cancel"),
            });
        }
    }
    if state.tracing {
        if let Some(pid) = state.trace_start_pid {
            return Some(match state.lang {
                Lang::Pt => format!("TRACE {pid}: s/q para parar (syscalls ao vivo)"),
                Lang::En => format!("TRACING {pid}: s/q to stop (live syscalls)"),
            });
        }
    }
    state.status_msg.clone()
}

/// The history pane has its own hints, pre-written at three widths because the
/// timeline keys do not degrade gracefully one token at a time.
fn history_status_line(state: &State, width: usize) -> String {
    let rec = if state.history.recording {
        "● REC"
    } else {
        "⏸ PAUSED"
    };
    let play = if state.history.playing {
        " [PLAYING]"
    } else {
        ""
    };
    let live = if state.history.is_live() {
        "LIVE"
    } else {
        "SCRUB"
    };

    let full = format!(
        "[7:HIST] {rec}{play} | {live} | < > step 1s | [ ] jump | Space play | 0 live | Tab metric ({}) | z span ({}) | e export | r rec | 1-6 panes | ? help | q quit",
        state.history.metric.label(),
        state.history.span.label()
    );
    if width == 0 || full.chars().count() <= width {
        return full;
    }
    let medium = format!(
        "[7:HIST] {rec}{play} | {live} | <> step | [] jump | Space play | 0 live | Tab metric | e export | ? help | q quit"
    );
    if medium.chars().count() <= width {
        return medium;
    }
    format!("[7:HIST] {rec}{play} | <> step | Space play | ? help | q quit")
}

/// The `[1:CPU + PROCS | CORES] ` style label that opens the status line.
fn pane_prefix(state: &State) -> &'static str {
    if !state.fullscreen {
        return "[DASHBOARD] ";
    }
    match state.pane {
        Pane::Cpu if state.cores_focused => "[1:CPU + PROCS | CORES] ",
        Pane::Cpu => "[1:CPU + PROCS | PROCESSES] ",
        Pane::Io => "[2:IO] ",
        Pane::Net => "[3:NET] ",
        Pane::Mem => "[4:MEM] ",
        Pane::Disks => "[5:DISKS] ",
        Pane::Gpu => "[6:GPU] ",
        Pane::History => "[7:HIST] ",
    }
}

/// Hints for the CPU pane with the core grid focused.
fn core_tokens(width: usize) -> Vec<String> {
    let mut tokens: Vec<String> = vec![
        "m menu".into(),
        "Tab procs".into(),
        "Enter filter core".into(),
        "arrows nav".into(),
    ];
    if width == 0 || width >= WIDTH_FOR_CORE_SORT_KEYS {
        tokens.push("p CPU".into());
        tokens.push("M MEM".into());
        tokens.push("z pause".into());
    }
    tokens
}

/// Hints for the CPU pane with the process table focused.
fn process_tokens(state: &State, width: usize) -> Vec<String> {
    let mut tokens: Vec<String> = vec![
        "m menu".into(),
        "Tab cores".into(),
        "y/Enter copy".into(),
        "←→ scroll".into(),
        "↑↓ nav".into(),
        "k kill".into(),
    ];
    if width == 0 || width >= WIDTH_FOR_PROC_SORT_KEYS {
        tokens.push("p CPU".into());
        tokens.push("M MEM".into());
    }
    tokens.push("/ search".into());
    if width == 0 || width >= WIDTH_FOR_TREE_AND_TRACE {
        tokens.push(format!("t tree{}", check(state.tree)));
        tokens.push("s trace".into());
    }
    if width == 0 || width >= WIDTH_FOR_THREAD_FILTERS {
        tokens.push(format!("H threads{}", check(state.show_threads)));
        tokens.push(format!("K kernel{}", check(state.show_kernel)));
        tokens.push("i reverse".into());
    }
    tokens
}

fn check(enabled: bool) -> &'static str {
    if enabled {
        " \u{2713}"
    } else {
        ""
    }
}

fn status_tokens(state: &State, width: usize) -> Vec<String> {
    let generic = || {
        vec![
            "m menu".to_string(),
            "1-7 panes".to_string(),
            "z pause".to_string(),
        ]
    };
    if !state.fullscreen {
        return generic();
    }
    match state.pane {
        Pane::Cpu if state.cores_focused => core_tokens(width),
        Pane::Cpu => process_tokens(state, width),
        _ => generic(),
    }
}

/// Appends as many hints as fit, always leaving room for the help/quit suffix.
fn fit_tokens(prefix: String, tokens: &[String], width: usize) -> String {
    if width == 0 {
        return format!("{prefix}{}{STATUS_SUFFIX}", tokens.join(" | "));
    }
    let available = width.saturating_sub(STATUS_SUFFIX.chars().count());
    let mut out = prefix;
    let mut first = true;
    for token in tokens {
        let sep = if first { "" } else { " | " };
        if out.chars().count() + sep.chars().count() + token.chars().count() > available {
            continue;
        }
        out.push_str(sep);
        out.push_str(token);
        first = false;
    }
    out.push_str(STATUS_SUFFIX);
    out
}

pub(super) fn status_line_for_width(state: &State, width: usize) -> String {
    if let Some(line) = status_line_override(state) {
        return line;
    }
    if state.fullscreen && state.pane == Pane::History {
        return history_status_line(state, width);
    }
    let core_filter = if state.fullscreen && state.pane == Pane::Cpu {
        state
            .core_filter
            .map(|c| format!("core filter {c} | Esc clears | "))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let prefix = format!(
        "{}{}{}{core_filter}",
        pane_prefix(state),
        if state.fullscreen { "[FULL] " } else { "" },
        if state.paused { "\u{23F8} PAUSED " } else { "" },
    );
    fit_tokens(prefix, &status_tokens(state, width), width)
}

#[cfg(test)]
mod tests {

    use super::*;

    /// The status line at whatever width the terminal happens to be.
    fn status_line(state: &State) -> String {
        status_line_for_width(state, 0)
    }

    #[test]
    fn status_line_shows_context() {
        let mut s = State::default();
        assert!(status_line(&s).contains("[DASHBOARD]"));
        s.fullscreen = true;
        assert!(status_line(&s).contains("[1:CPU + PROCS | PROCESSES]"));
        s.cores_focused = true;
        assert!(status_line(&s).contains("[1:CPU + PROCS | CORES]"));
        s.paused = true;
        assert!(status_line(&s).contains("PAUSED"));
        s.core_filter = Some(3);
        assert!(status_line(&s).contains("core filter 3"));
    }

    #[test]
    fn status_line_fits_given_width() {
        let s = State::default();
        let st100 = status_line_for_width(&s, 100);
        assert!(st100.chars().count() <= 100);
        assert!(st100.contains("1-7 panes"));
        assert!(st100.contains("help"));

        let st140 = status_line_for_width(&s, 140);
        assert!(st140.chars().count() <= 140);
        assert!(st140.contains("1-7 panes"));
        assert!(st140.contains("help"));
    }

    #[test]
    fn history_status_line_contains_help() {
        let s = State {
            pane: Pane::History,
            fullscreen: true,
            ..State::default()
        };
        assert!(status_line_for_width(&s, 80).contains("help"));
        assert!(status_line_for_width(&s, 140).contains("help"));
    }
}

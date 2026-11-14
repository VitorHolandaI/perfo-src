//! Copying a full command line out of the TUI.
//!
//! Each helper runs on a keypress, so none may outlive its budget: a wedged
//! one would freeze the interface while the user waits for a copy.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::bounded_child::wait_bounded;

/// base64 turns three input bytes into four output characters.
const B64_IN_CHUNK: usize = 3;
const B64_OUT_CHUNK: usize = 4;

pub(super) fn to_base64(data: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(B64_IN_CHUNK) * B64_OUT_CHUNK);
    for chunk in data.chunks(B64_IN_CHUNK) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(B64[(triple >> 18) & 0x3F] as char);
        result.push(B64[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            result.push(B64[(triple >> 6) & 0x3F] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(B64[triple & 0x3F] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// How long one clipboard helper may take. It runs on a keypress, so a wedged
/// helper would freeze the terminal UI; `wl-copy` normally answers in under a
/// millisecond here because it forks a server and the parent exits.
const HELPER_BUDGET: Duration = Duration::from_secs(2);

/// The helpers tried in order, each with the arguments that make it take the
/// selection on stdin.
const HELPERS: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["-b"]),
];

/// Feeds `text` to one helper and reports whether it took it.
///
/// Absent from this machine, failing, and not answering within `budget` are all
/// the same answer to the caller: try the next one.
fn offer_to_helper(program: &str, args: &[&str], text: &str, budget: Duration) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
        // Dropped here on purpose: the helper reads to end of input, so it
        // cannot exit while this pipe is open.
    }
    wait_bounded(&mut child, budget).is_some_and(|status| status.success())
}

pub(super) fn copy_to_clipboard(text: &str) {
    copy_with_helpers(text, HELPERS, HELPER_BUDGET);
}

/// The helper list is a parameter so a test can hand it a program that never
/// exits and check the budget is honoured.
fn copy_with_helpers(text: &str, helpers: &[(&str, &[&str])], budget: Duration) -> bool {
    let took_it = helpers
        .iter()
        .any(|(program, args)| offer_to_helper(program, args, text, budget));

    // 4. Always emit OSC 52 sequence to stdout (works across modern terminals & SSH)
    let b64 = to_base64(text.as_bytes());
    let _ = std::io::stdout().write_all(format!("\x1b]52;c;{b64}\x07").as_bytes());
    let _ = std::io::stdout().flush();
    took_it
}

pub(super) fn get_process_full_cmd(pid: u32) -> String {
    if let Ok(content) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        if !content.is_empty() {
            let mut parts = Vec::new();
            for part in content.split(|&b| b == 0) {
                if !part.is_empty() {
                    parts.push(String::from_utf8_lossy(part).to_string());
                }
            }
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
    }
    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        let trimmed = comm.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format!("[{pid}]")
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    /// A helper that never reads its input and never exits used to hold the
    /// keypress through a bare `wait()`. The budget has to end it.
    #[test]
    fn a_helper_that_never_exits_does_not_hold_the_keypress() {
        let helpers: &[(&str, &[&str])] = &[("sleep", &["60"])];
        let started = Instant::now();

        let took_it = copy_with_helpers("anything", helpers, Duration::from_millis(150));

        assert!(
            !took_it,
            "a helper that timed out did not take the selection"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}, so the budget was not honoured",
            started.elapsed()
        );
    }

    /// A helper missing from this machine is not an error: the next one is
    /// tried, and that is how one build serves Wayland and X11 both.
    #[test]
    fn an_absent_helper_falls_through_to_the_next() {
        let helpers: &[(&str, &[&str])] = &[("perfo-no-such-clipboard-helper", &[]), ("cat", &[])];

        assert!(
            copy_with_helpers("anything", helpers, Duration::from_secs(5)),
            "cat reads stdin and exits 0, so it should report success"
        );
    }

    #[test]
    fn a_helper_that_fails_is_not_counted_as_a_copy() {
        let helpers: &[(&str, &[&str])] = &[("false", &[])];
        assert!(!copy_with_helpers(
            "anything",
            helpers,
            Duration::from_secs(5)
        ));
    }

    use super::*;
    use crate::tui::State;

    use crate::tui::keys::handle_key;
    use crossterm::event::{KeyCode, KeyModifiers};
    #[test]
    fn base64_encoding_works() {
        assert_eq!(to_base64(b""), "");
        assert_eq!(to_base64(b"f"), "Zg==");
        assert_eq!(to_base64(b"fo"), "Zm8=");
        assert_eq!(to_base64(b"foo"), "Zm9v");
        assert_eq!(to_base64(b"perfo monitor"), "cGVyZm8gbW9uaXRvcg==");
    }

    #[test]
    fn copy_key_sets_status_message() {
        let mut s = State {
            selected_pid: Some(std::process::id()),
            fullscreen: true,
            ..State::default()
        };
        handle_key(&mut s, &[], KeyCode::Char('y'), KeyModifiers::empty(), None);
        assert!(s.status_msg.is_some());
        assert!(
            s.status_msg.as_ref().unwrap().contains("copiado")
                || s.status_msg.as_ref().unwrap().contains("Copied")
        );
    }
}

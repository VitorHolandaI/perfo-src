//! Writing a history report to the user's home directory.

use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::paths::get_home_dir;
use crate::bounded_child::wait_bounded;

/// How long the desktop notification may take. It is a courtesy, so the export
/// never waits long for it; a host with no notification daemon fails fast.
const NOTIFY_BUDGET: Duration = Duration::from_secs(2);

/// Tells the desktop the export finished, and reaps the helper.
///
/// Spawning without waiting left a zombie for the life of the process, one per
/// export, because nothing ever collected the exit status. The program is a
/// parameter so a test can hand it one that exits.
fn announce(program: &str, basename: &str) -> bool {
    let Ok(mut child) = Command::new(program)
        .args([
            "-a",
            "Perfo",
            "History Exported",
            &format!("Saved: ~/{basename}.{{txt,json}}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    wait_bounded(&mut child, NOTIFY_BUDGET).is_some_and(|status| status.success())
}

// qual:allow(test_quality, untested) reason: "consumes the process's real stdin; the payload shape is covered by the store tests"
pub fn export_from_stdin(basename: &str) -> io::Result<(String, String)> {
    let mut buf = String::new();
    let n = io::stdin().read_line(&mut buf)?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Empty export input",
        ));
    }
    let trimmed = buf.trim();
    let raw = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        buf
    } else {
        let _ = io::stdin().read_to_string(&mut buf);
        buf
    };
    if raw.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Empty export input",
        ));
    }
    let val: Value = serde_json::from_str(raw.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let home = get_home_dir();
    let txt_name = format!("{}.txt", basename);
    let json_name = format!("{}.json", basename);
    let txt_path = Path::new(&home).join(&txt_name);
    let json_path = Path::new(&home).join(&json_name);

    if let Some(txt) = val.get("text").and_then(|v| v.as_str()) {
        fs::write(&txt_path, txt)?;
    }
    if let Some(json_val) = val.get("json") {
        let json_str = match json_val {
            Value::String(s) => s.clone(),
            _ => serde_json::to_string_pretty(json_val).map_err(io::Error::other)?,
        };
        fs::write(&json_path, json_str)?;
    }

    announce("notify-send", basename);

    Ok((
        txt_path.to_string_lossy().to_string(),
        json_path.to_string_lossy().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The notification used to be spawned and never waited on, which left a
    /// zombie for the life of the process, one per export.
    #[test]
    fn the_notification_helper_is_reaped() {
        assert!(
            announce("true", "report"),
            "a helper that exits 0 succeeded"
        );
    }

    /// A host with no notification daemon has no `notify-send`, and an export
    /// is not a failure because nobody could be told about it.
    #[test]
    fn an_absent_notifier_is_not_an_error() {
        assert!(!announce("perfo-no-such-notifier", "report"));
    }

    /// The budget itself is covered by `bounded_child`, with a child that
    /// really does hang. It cannot be exercised from here: `announce` sends
    /// fixed arguments, and a program that ignores them and still sleeps is not
    /// available, so a test using `sleep` would pass because `sleep` rejects
    /// `-a` rather than because any deadline was honoured.
    #[test]
    fn a_notifier_that_fails_is_not_reported_as_sent() {
        assert!(!announce("false", "report"));
    }
}

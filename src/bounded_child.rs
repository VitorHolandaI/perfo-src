//! Waiting for a child process with a budget.
//!
//! Two places spawn a short-lived helper and must not be held by one that
//! never answers: the `nvidia-smi` sampler, where a wedged driver kept the
//! collector from its first snapshot, and the clipboard, where `wl-copy`,
//! `xclip` or `xsel` run on a keypress. Both used a bare `wait()`, which has
//! no deadline at all.

use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// How often the wait checks, which bounds how long it overshoots its budget.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Waits for `child` for at most `budget`, then kills and reaps it.
///
/// `None` means it did not finish in time, which is the only honest answer
/// about a helper that may never return.
///
/// ```ignore
/// let mut child = Command::new("wl-copy").spawn()?;
/// if wait_bounded(&mut child, Duration::from_secs(2)).is_some_and(|s| s.success()) {
///     // the clipboard took it
/// }
/// ```
pub(crate) fn wait_bounded(child: &mut Child, budget: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + budget;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Err(_) => return None,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn spawn(program: &str, args: &[&str]) -> Child {
        Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("{program} is not available: {e}"))
    }

    /// The whole point: a child that never answers must not hold the caller.
    #[test]
    fn a_child_that_never_exits_is_killed_at_the_budget() {
        let mut child = spawn("sleep", &["60"]);
        let started = Instant::now();

        assert!(wait_bounded(&mut child, Duration::from_millis(150)).is_none());

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}, so the budget was not honoured",
            started.elapsed()
        );
        // Reaped, not left as a zombie.
        assert!(matches!(child.try_wait(), Ok(Some(_))));
    }

    #[test]
    fn a_child_that_finishes_reports_its_status() {
        let mut child = spawn("true", &[]);
        assert!(wait_bounded(&mut child, Duration::from_secs(5)).is_some_and(|s| s.success()));
    }

    #[test]
    fn a_failing_child_reports_failure_rather_than_a_timeout() {
        let mut child = spawn("false", &[]);
        let status = wait_bounded(&mut child, Duration::from_secs(5));
        assert!(status.is_some(), "it exited, so this is not a timeout");
        assert!(!status.unwrap().success());
    }
}

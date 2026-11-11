//! A collector that outlives its client keeps sampling once a second forever.
//! The widget drives `perfo stream` over stdin, so the client's end of that
//! pipe closing is the only timely evidence it went away: the stream's own
//! writes do not fail until something reads the other side, and an orphan
//! whose output nobody drains never notices.
//!
//! These drive the real binary, because the bug was in how the process ends
//! rather than in anything a library call can show.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Waits for a child to exit, so a hang fails the test instead of parking the
/// suite forever. Returns `None` when it was still running at the deadline.
fn wait_until_exit(child: &mut Child, patience: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait failed") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

fn stream_command() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_perfo"));
    cmd.args(["stream", "--json", "--summary"]);
    cmd
}

#[test]
fn closing_the_control_pipe_ends_the_stream() {
    let mut child = stream_command()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start perfo stream");

    // Dropping our write end is exactly what a widget that died without
    // closing its pipe leaves behind. Stdout stays open and undrained, so a
    // broken-pipe write cannot be what ends this process.
    drop(child.stdin.take());

    let status = wait_until_exit(&mut child, Duration::from_secs(10));
    let Some(status) = status else {
        let _ = child.kill();
        panic!("the collector outlived its client: stdin closed and it kept running");
    };
    assert!(
        status.success(),
        "losing a client is an ordinary end, not a failure: {status:?}"
    );
}

#[test]
fn a_stream_with_no_client_on_stdin_keeps_sampling() {
    // `< /dev/null` reads EOF at once. That is not a client going away, and a
    // script redirecting stdin must still get its samples.
    let mut child = stream_command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start perfo stream");

    // Drain for as long as the child lives. Stopping early would close this
    // end and hand it the broken pipe it is supposed to exit on, which would
    // pass this test for the wrong reason.
    let mut stdout = child.stdout.take().expect("no stdout");
    let written = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&written);
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            counter.fetch_add(n, Ordering::Relaxed);
        }
    });

    if let Some(status) = wait_until_exit(&mut child, Duration::from_secs(4)) {
        panic!("exited on an EOF that no client caused: {status:?}");
    }
    assert!(
        written.load(Ordering::Relaxed) > 0,
        "no samples were written before the deadline"
    );
    child.kill().expect("failed to stop the collector");
    let _ = child.wait();
    reader.join().expect("reader thread panicked");
}

/// `perfo stream --json | head -1` is the shape of every quick look at this
/// command. The default `println!` panics on the broken pipe that leaves, so a
/// normal shell idiom printed a Rust backtrace and exited 101.
#[test]
fn a_consumer_that_stops_reading_is_not_a_crash() {
    let mut child = stream_command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start perfo stream");

    let mut stdout = child.stdout.take().expect("no stdout");
    let mut first = [0u8; 16];
    stdout.read_exact(&mut first).expect("no sample arrived");
    drop(stdout);

    let status = match wait_until_exit(&mut child, Duration::from_secs(10)) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            panic!("kept writing to a pipe nobody reads");
        }
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("no stderr")
        .read_to_string(&mut stderr)
        .expect("failed to read stderr");
    assert!(
        !stderr.contains("panicked"),
        "a closed consumer produced a panic: {stderr}"
    );
    assert!(status.success(), "expected a clean exit, got {status:?}");
}

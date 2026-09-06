pub mod errnos;
pub mod syscalls;

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Once};

use libc::{c_int, user_regs_struct};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static TRACE_TID: AtomicUsize = AtomicUsize::new(0);
static SIGINT_INSTALLED: Once = Once::new();

extern "C" fn on_sigint(_: c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn install_sigint() {
    SIGINT_INSTALLED.call_once(|| unsafe {
        // SAFETY: sets the process-wide SIGINT handler once; on_sigint only
        // writes to a static AtomicBool. The pointer cast is the libc::signal
        // contract on Linux (sa_handler is usize-typed there).
        libc::signal(libc::SIGINT, on_sigint as extern "C" fn(c_int) as usize);
    });
}

/// Signals the currently running tracer thread (if any) to stop and detach.
/// Uses pthread_kill so only the tracer thread's blocked waitpid gets EINTR.
pub fn stop_current_trace() {
    let tid = TRACE_TID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            // SAFETY: TRACE_TID holds a live pthread_t of our own tracer
            // thread, which is blocked in waitpid and handles EINTR.
            libc::pthread_kill(tid as libc::pthread_t, libc::SIGINT);
        }
    }
}

const STOPPED: c_int = 0x7f;
/// With PTRACE_O_TRACESYSGOOD, syscall stops arrive as SIGTRAP | 0x80.
const SYSCALL_STOP: c_int = libc::SIGTRAP | 0x80;

fn wifexited(s: c_int) -> bool {
    s & 0x7f == 0
}
fn wifsignaled(s: c_int) -> bool {
    s & 0x7f != 0 && s & 0x7f != STOPPED
}
fn wifstopped(s: c_int) -> bool {
    s & 0xff == STOPPED
}
fn wstopsig(s: c_int) -> c_int {
    (s >> 8) & 0xff
}
fn wtermsig(s: c_int) -> c_int {
    s & 0x7f
}

/// Waits for a tracee stop. Returns true when interrupted by SIGINT while
/// blocked (caller should break the loop and detach), or the wait error.
fn wait_tracee(pid: i32, status: &mut c_int) -> io::Result<bool> {
    loop {
        let r = unsafe { libc::waitpid(pid, status, libc::__WALL) };
        if r == -1 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return Ok(true);
                }
                continue;
            }
            return Err(e);
        }
        return Ok(false);
    }
}

fn get_regs(pid: i32) -> io::Result<user_regs_struct> {
    // SAFETY: zeroed() is a valid representation for user_regs_struct, and the
    // kernel fills it via PTRACE_GETREGS before we read it.
    let mut regs: user_regs_struct = unsafe { std::mem::zeroed() };
    let r = unsafe {
        // SAFETY: pid is a live traced child/attached process; &mut regs stays
        // valid for the duration of the call.
        libc::ptrace(
            libc::PTRACE_GETREGS,
            pid,
            0,
            &mut regs as *mut _ as *mut libc::c_void,
        )
    };
    if r == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(regs)
}

fn ptrace_checked(
    request: libc::c_uint,
    pid: libc::pid_t,
    addr: *mut libc::c_void,
    data: *mut libc::c_void,
) -> io::Result<()> {
    // SAFETY: ptrace owns the request-specific interpretation of these
    // pointers; callers pass valid buffers or null for requests without one.
    let result = unsafe { libc::ptrace(request, pid, addr, data) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn fmt_arg(v: u64) -> String {
    if v < 10 {
        format!("{v}")
    } else {
        format!("0x{v:x}")
    }
}

/// Reads a C string from the tracee's memory via PTRACE_PEEKDATA.
/// Returns None when the memory is unreadable or not a printable string.
fn read_cstr(pid: i32, addr: u64) -> Option<String> {
    let mut out: Vec<u8> = Vec::new();
    let mut cur = addr;
    for _ in 0..64 {
        let word = unsafe {
            // SAFETY: pid is stopped in a ptrace syscall-stop, so the tracee's
            // memory is quiescent while we read one word at cur.
            libc::ptrace(libc::PTRACE_PEEKDATA, pid, cur as *mut libc::c_void, 0)
        };
        if word == -1 && io::Error::last_os_error().raw_os_error() != Some(0) {
            return None;
        }
        for b in (word as u64).to_ne_bytes() {
            if b == 0 {
                return String::from_utf8(out).ok();
            }
            if !(0x20..0x7f).contains(&b) {
                return None;
            }
            out.push(b);
        }
        cur = cur.checked_add(8)?;
    }
    None
}

const AT_FDCWD: u64 = 0xffffffffffffff9c;

/// For known syscalls, which argument is a path/string in the tracee.
fn string_arg_index(name: &str) -> Option<usize> {
    match name {
        "open" | "creat" | "unlink" | "chdir" | "chmod" | "chown" | "lchown" | "stat" | "lstat"
        | "access" | "readlink" | "mkdir" | "rmdir" | "rename" | "link" | "symlink"
        | "truncate" | "mknod" | "mount" | "umount" | "umount2" | "swapon" | "swapoff" | "acct"
        | "utime" | "utimes" | "statfs" | "fstatfs" | "getxattr" | "setxattr" | "listxattr"
        | "removexattr" | "lgetxattr" | "lsetxattr" | "inotify_add_watch" | "quotactl"
        | "execve" => Some(0),
        "openat" | "newfstatat" | "unlinkat" | "readlinkat" | "mkdirat" | "renameat"
        | "renameat2" | "linkat" | "symlinkat" | "fchmodat" | "faccessat" | "execveat"
        | "statx" | "mknodat" | "fchownat" | "utimensat" | "name_to_handle_at"
        | "open_by_handle_at" => Some(1),
        _ => None,
    }
}

/// For the *at family, which argument is a directory fd (AT_FDCWD-able).
fn dirfd_arg_index(name: &str) -> Option<usize> {
    match name {
        "openat" | "newfstatat" | "unlinkat" | "readlinkat" | "mkdirat" | "renameat"
        | "renameat2" | "linkat" | "symlinkat" | "fchmodat" | "faccessat" | "execveat"
        | "statx" | "mknodat" | "fchownat" | "utimensat" | "name_to_handle_at" => Some(0),
        _ => None,
    }
}

fn fmt_ret(ret: i64) -> String {
    if ret < 0 {
        match errnos::name((-ret) as usize) {
            Some(name) => format!("{ret} {name}"),
            None => format!("{ret}"),
        }
    } else {
        format!("{ret}")
    }
}

fn fmt_args(pid: i32, regs: &user_regs_struct, name: &str) -> String {
    let raw = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9];
    let mut parts: Vec<String> = Vec::new();
    for (i, v) in raw.iter().enumerate() {
        if string_arg_index(name) == Some(i) {
            if let Some(s) = read_cstr(pid, *v) {
                parts.push(format!("\"{s}\""));
                continue;
            }
        }
        if dirfd_arg_index(name) == Some(i) && *v == AT_FDCWD {
            parts.push("AT_FDCWD".into());
            continue;
        }
        parts.push(fmt_arg(*v));
    }
    parts.join(", ")
}

/// Core ptrace loop: emits one complete line per traced syscall via `emit`.
/// Stops (and detaches) when INTERRUPTED is set or the tracee exits.
fn run_loop(pid: i32, filter: Option<&str>, emit: &mut dyn FnMut(String)) -> io::Result<()> {
    let mut pending: Option<String> = None;
    let mut status: c_int = 0;
    // Syscall stops strictly alternate entry/exit. The kernel sets rax=-ENOSYS
    // at entry and leaves orig_rax as the syscall number on modern kernels, so
    // classify the first stop by rax and toggle afterwards.
    let mut at_entry: Option<bool> = None;
    let mut resume_signal: Option<c_int> = None;

    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            break;
        }
        let signal = resume_signal
            .take()
            .map(|signal| signal as *mut libc::c_void)
            .unwrap_or(std::ptr::null_mut());
        ptrace_checked(libc::PTRACE_SYSCALL, pid, std::ptr::null_mut(), signal)?;
        if wait_tracee(pid, &mut status)? {
            break;
        }

        if wifexited(status) {
            if pending.take().is_some() {
                emit(" = <interrupted>".into());
            }
            emit(format!("+++ exited with {} +++", status >> 8));
            break;
        }
        if wifsignaled(status) {
            if pending.take().is_some() {
                emit(" = <interrupted>".into());
            }
            emit(format!("+++ killed by signal {} +++", wtermsig(status)));
            break;
        }
        if !wifstopped(status) {
            continue;
        }

        let sig = wstopsig(status);
        if sig == SYSCALL_STOP {
            let regs = get_regs(pid)?;
            let entry = match at_entry {
                Some(e) => !e,
                None => regs.rax as i64 == -38, // -ENOSYS placeholder on entry
            };
            at_entry = Some(entry);
            if !entry {
                // syscall exit stop
                if let Some(name) = pending.take() {
                    emit(format!("{name} = {}", fmt_ret(regs.rax as i64)));
                }
            } else {
                let nr = regs.orig_rax as usize;
                let name = syscalls::name(nr).unwrap_or("syscall?");
                let shown = filter.is_none_or(|f| name.contains(f));
                if shown {
                    if pending.take().is_some() {
                        emit(" = <interrupted>".into());
                    }
                    pending = Some(format!("{name}({})", fmt_args(pid, &regs, name)));
                }
            }
        } else {
            // Signal stop: forward it on the next resume, after checking the
            // stop status and avoiding a second ptrace resume in this cycle.
            resume_signal = Some(sig);
        }
    }

    if let Some(name) = pending.take() {
        emit(format!("{name} = <interrupted>"));
    }
    if let Err(error) = ptrace_checked(
        libc::PTRACE_DETACH,
        pid,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    ) {
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

fn attach_preamble(pid: i32) -> io::Result<()> {
    unsafe {
        // SAFETY: PTRACE_SEIZE (no stop, non-blocking attach) on a live pid;
        // EPERM when the yama policy forbids tracing non-children.
        let r = libc::ptrace(
            libc::PTRACE_SEIZE,
            pid,
            0,
            libc::PTRACE_O_TRACESYSGOOD as *mut libc::c_void,
        );
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
        // Get an initial stop so we can start issuing PTRACE_SYSCALL.
        ptrace_checked(
            libc::PTRACE_INTERRUPT,
            pid,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )?;
    }
    let mut status: c_int = 0;
    if wait_tracee(pid, &mut status)? || !wifstopped(status) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "tracee did not stop after ptrace attach",
        ));
    }
    Ok(())
}

/// CLI: trace an existing process. Ctrl+C detaches and lets it continue.
pub fn attach(pid: i32, filter: Option<&str>) -> io::Result<()> {
    install_sigint();
    INTERRUPTED.store(false, Ordering::SeqCst);
    attach_preamble(pid)?;
    let stdout = io::stdout();
    let mut emit = |s: String| {
        let _ = writeln!(stdout.lock(), "{s}");
    };
    run_loop(pid, filter, &mut emit)
}

/// CLI: spawn a command with tracing enabled (PTRACE_TRACEME) and trace it.
pub fn spawn(cmd: &[String], filter: Option<&str>) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if cmd.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "trace command is empty (expected executable and optional arguments)",
        ));
    }
    install_sigint();
    INTERRUPTED.store(false, Ordering::SeqCst);
    let mut child = Command::new(&cmd[0]);
    child
        .args(&cmd[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    unsafe {
        child.pre_exec(|| {
            // SAFETY: PTRACE_TRACEME uses no pointed-to data and runs in the
            // child before exec replaces its address space.
            let result = libc::ptrace(
                libc::PTRACE_TRACEME,
                0,
                0,
                std::ptr::null_mut::<libc::c_void>(),
            );
            if result == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = child.spawn()?;
    let pid = child.id() as i32;

    // Child stops with SIGTRAP right after exec.
    let mut status: c_int = 0;
    if wait_tracee(pid, &mut status)? || !wifstopped(status) {
        return Err(io::Error::other("tracee did not stop after exec"));
    }
    // TRACEME doesn't take options; enable TRACESYSGOOD now so syscall stops
    // arrive as SIGTRAP|0x80 instead of plain SIGTRAP.
    ptrace_checked(
        libc::PTRACE_SETOPTIONS,
        pid,
        std::ptr::null_mut(),
        libc::PTRACE_O_TRACESYSGOOD as *mut libc::c_void,
    )?;
    let stdout = io::stdout();
    let mut emit = |s: String| {
        let _ = writeln!(stdout.lock(), "{s}");
    };
    run_loop(pid, filter, &mut emit)
}

/// Thread-mode tracer for the TUI: emits lines through `tx`. Stop it with
/// `stop_current_trace()` (pthread_kill), which interrupts the blocked waitpid.
pub fn attach_stream(pid: i32, filter: Option<&str>, tx: mpsc::Sender<String>) -> io::Result<()> {
    install_sigint();
    INTERRUPTED.store(false, Ordering::SeqCst);
    // SAFETY: pthread_self() always returns a valid handle of the calling
    // thread, which stays alive while we store it (same thread, no move).
    TRACE_TID.store(unsafe { libc::pthread_self() } as usize, Ordering::SeqCst);
    let result = (|| {
        attach_preamble(pid)?;
        let mut emit = |s: String| {
            let _ = tx.send(s);
        };
        run_loop(pid, filter, &mut emit)
    })();
    TRACE_TID.store(0, Ordering::SeqCst);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syscall_names_known_entries() {
        assert_eq!(syscalls::name(0), Some("read"));
        assert_eq!(syscalls::name(1), Some("write"));
        assert_eq!(syscalls::name(60), Some("exit"));
        assert_eq!(syscalls::name(9999), None);
    }

    #[test]
    fn errno_names_known_entries() {
        assert_eq!(errnos::name(1), Some("EPERM"));
        assert_eq!(errnos::name(2), Some("ENOENT"));
        assert_eq!(errnos::name(13), Some("EACCES"));
        assert_eq!(errnos::name(9999), None);
    }

    #[test]
    fn fmt_ret_renders_errno() {
        assert_eq!(fmt_ret(-2), "-2 ENOENT");
        assert_eq!(fmt_ret(42), "42");
    }

    #[test]
    fn dirfd_index_covers_at_family() {
        assert_eq!(dirfd_arg_index("openat"), Some(0));
        assert_eq!(dirfd_arg_index("read"), None);
    }

    #[test]
    fn string_arg_index_path_syscalls() {
        assert_eq!(string_arg_index("open"), Some(0));
        assert_eq!(string_arg_index("write"), None);
    }

    #[test]
    fn wait_tracee_returns_error_for_unknown_pid() {
        let mut status = 0;
        assert!(wait_tracee(i32::MAX, &mut status).is_err());
    }

    #[test]
    fn spawn_rejects_empty_commands() {
        assert!(spawn(&[], None).is_err());
    }
}

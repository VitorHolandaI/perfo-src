//! Per-process facts read straight from /proc, which sysinfo does not expose.

use serde::Serialize;

/// /proc/<pid>/stat: field 39 (last processor) is the 36th whitespace token
/// after the closing `)` of the comm field.
pub(crate) const LAST_CPU_STAT_FIELD: usize = 36;

#[derive(Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    /// Kernel process name, independent of command-line arguments.
    pub name: String,
    pub ppid: Option<u32>,
    /// Owning process pid when this entry is a thread (None for real processes).
    pub owner: Option<u32>,
    pub is_kernel: bool,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_bytes: u64,
    pub cmd: String,
    /// Index of the CPU this process last ran on (from /proc/<pid>/stat field 39).
    pub last_cpu: Option<u32>,
    /// Bytes actually reaching the storage layer per second (from
    /// /proc/<pid>/io read_bytes/write_bytes deltas). Zero when unreadable
    /// (yama/other-user) or idle.
    pub read_bps: u64,
    pub write_bps: u64,
    /// Bytes reaching the storage layer in the current window (see
    /// IO_WINDOW_SECS); "who hammered the disk lately".
    pub win_read_bytes: u64,
    pub win_write_bytes: u64,
}

/// Parses /proc/<pid>/io: (read_bytes, write_bytes): bytes that actually
/// reached the storage layer (submit_bio), unlike rchar/wchar which count
/// syscall bytes including page-cache hits.
pub(crate) fn proc_io_from(raw: &str) -> (u64, u64) {
    let mut out = (0, 0);
    for line in raw.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        let Some(v) = val.parse::<u64>().ok() else {
            continue;
        };
        match key {
            "read_bytes:" => out.0 = v,
            "write_bytes:" => out.1 = v,
            _ => {}
        }
    }
    out
}

pub(crate) fn proc_io_of(pid: u32) -> Option<(u64, u64)> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    Some(proc_io_from(&raw))
}

/// (iowait_ms, total_ms) from the aggregate "cpu " line of /proc/stat.
pub(crate) fn stat_iowait_from(raw: &str) -> (u64, u64) {
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("cpu ") {
            let nums: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if nums.len() >= 5 {
                // user nice system idle iowait irq ...
                let total: u64 = nums.iter().sum();
                return (nums[4], total);
            }
        }
    }
    (0, 0)
}

pub(crate) fn iowait_percent(previous: (u64, u64), current: (u64, u64)) -> f32 {
    let waited = current.0.saturating_sub(previous.0);
    let total = current.1.saturating_sub(previous.1);
    if total == 0 {
        0.0
    } else {
        crate::units::percent_of(waited, total)
    }
}

/// Last-run CPU from a /proc/<pid>/stat line. Field 39 (processor index) is
/// the 36th whitespace token after the closing `)` of the comm field (which
/// may itself contain spaces and parens).
pub(crate) fn last_cpu_from_stat(raw: &str) -> Option<u32> {
    let close = raw.rfind(')')?;
    raw[close + 1..]
        .split_whitespace()
        .nth(LAST_CPU_STAT_FIELD)?
        .parse()
        .ok()
}

pub(crate) fn last_cpu_of(pid: u32) -> Option<u32> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    last_cpu_from_stat(&raw)
}

/// Resolve a uid to a username via NSS (getpwuid_r), "?" on failure.
///
/// `users` crate (0.11) has RUSTSEC-2023-0059 (unsound) and is
/// unmaintained: this drops that dependency entirely.
pub(crate) fn user_name_of(uid: u32) -> String {
    let mut buf = [0u8; 256];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: buf lives for the call, getpwuid_r fills passwd/buf and stores
    // the resolved entry in `result`; return value 0 = success.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr() as *mut i8,
            buf.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() && !pwd.pw_name.is_null() {
        // SAFETY: pw_name points into buf, NUL-terminated by getpwuid_r.
        unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_string_lossy()
            .into_owned()
    } else {
        "?".into()
    }
}

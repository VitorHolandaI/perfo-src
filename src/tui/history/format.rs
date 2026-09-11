//! Names and timestamps as the history views print them.

pub(super) fn clean_process_name(name: &str, cmd: &str, pid: u32) -> String {
    let raw = if !name.is_empty() {
        name
    } else {
        cmd.split_whitespace().next().unwrap_or("")
    };
    let trimmed = raw.trim_matches(&['"', '\''][..]);
    let base = match trimmed.rfind('/') {
        Some(idx) => &trimmed[idx + 1..],
        None => trimmed,
    };
    if base.is_empty() {
        pid.to_string()
    } else {
        base.to_string()
    }
}

pub(super) fn format_local_time(time: std::time::SystemTime) -> String {
    let dur = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let sec = dur.as_secs() as libc::time_t;
    unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&sec, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

pub(crate) fn format_local_datetime(time: std::time::SystemTime) -> String {
    let dur = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let sec = dur.as_secs() as libc::time_t;
    unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&sec, &mut tm);
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

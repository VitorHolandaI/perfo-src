//! Human-readable labels for durations and recording timestamps.

use std::time::SystemTime;

pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        let rem = seconds % 60;
        if rem > 0 {
            format!("{}m {}s", seconds / 60, rem)
        } else {
            format!("{}m", seconds / 60)
        }
    } else {
        let h = seconds / 3600;
        let m = (seconds % 3600) / 60;
        if m > 0 {
            format!("{}h {}m", h, m)
        } else {
            format!("{}h", h)
        }
    }
}

pub(super) fn local_datetime_strings() -> (String, String, String) {
    let now = SystemTime::now();
    let dur = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let sec = dur.as_secs() as libc::time_t;
    unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&sec, &mut tm);
        let date_str = format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        );
        let time_str = format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec);
        let id_str = format!(
            "rec-{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        );
        (date_str, time_str, id_str)
    }
}

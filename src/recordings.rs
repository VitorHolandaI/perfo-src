use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Debug)]
pub struct RecordingMetadata {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub date: String,
    pub time: String,
    pub duration: String,
    pub duration_seconds: u64,
    pub sample_count: usize,
    pub metric_focus: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RecordingPayload {
    pub id: String,
    pub filename: String,
    pub date: String,
    pub time: String,
    pub duration_seconds: u64,
    pub duration_label: String,
    pub metric_focus: String,
    pub sample_count: usize,
    pub samples: Value,
}

pub fn get_home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub const DEFAULT_MAX_RECORDINGS: usize = 5;

/// Recordings are persistent user data, so they follow the XDG data directory
/// rather than a shell profile: a GUI widget is never started from a login
/// shell and so never inherits one.
pub fn default_recordings_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| get_home_dir().join(".local/share"))
        .join("perfo/recordings")
}

pub fn get_config() -> (PathBuf, usize) {
    let rec_dir = std::env::var_os("PERFO_RECORDINGS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_recordings_dir);

    let _ = fs::create_dir_all(&rec_dir);

    let max_recs = std::env::var("PERFO_MAX_RECORDINGS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_RECORDINGS)
        .max(1);

    (rec_dir, max_recs)
}

/// Reject ids that would escape the recordings directory. Ids normally come
/// from `record list`, but every subcommand is reachable from the CLI too.
fn sanitize_recording_id(target: &str) -> io::Result<&str> {
    if target.is_empty() || target.contains('/') || target == "." || target == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid recording id: {target:?}"),
        ));
    }
    Ok(target)
}

/// Map an id or explicit path to the file it names, without touching the disk.
fn recording_path_for(target: &str, rec_dir: &Path) -> io::Result<PathBuf> {
    if target.ends_with(".json") {
        if let Some(rest) = target.strip_prefix("~/") {
            Ok(get_home_dir().join(rest))
        } else if target.contains('/') {
            Ok(PathBuf::from(target))
        } else {
            Ok(rec_dir.join(target))
        }
    } else {
        Ok(rec_dir.join(format!("{}.json", sanitize_recording_id(target)?)))
    }
}

/// True when `path` sits directly inside `dir`. Recordings are stored flat, so
/// an exact parent match is enough and `..` cannot slip through canonicalize.
fn is_inside(path: &Path, dir: &Path) -> bool {
    let (Ok(base), Some(parent)) = (dir.canonicalize(), path.parent()) else {
        return false;
    };
    if parent.as_os_str().is_empty() {
        return false;
    }
    parent.canonicalize().map(|p| p == base).unwrap_or(false)
}

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

pub fn prune_recordings(rec_dir: &Path, max_recs: usize) {
    if let Ok(entries) = fs::read_dir(rec_dir) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| {
                            s.starts_with("rec-")
                                && s.ends_with(".json")
                                && !s.ends_with(".timeline.json")
                        })
                        .unwrap_or(false)
            })
            .collect();

        // Sort descending by name (rec-YYYYMMDD-HHMMSS.json) which is chronological
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        if files.len() > max_recs {
            for f in &files[max_recs..] {
                let _ = fs::remove_file(f);
                let mut p = f.clone();
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("rec")
                    .to_string();
                p.set_file_name(format!("{}.timeline.json", stem));
                let _ = fs::remove_file(&p);
                p.set_file_name(format!("{}.offsets.bin", stem));
                let _ = fs::remove_file(&p);
            }
        }
    }
}

fn local_datetime_strings() -> (String, String, String) {
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

fn read_recording_metadata(path: &Path) -> Option<RecordingMetadata> {
    // Fast path: read first 4096 bytes without loading multi-megabyte samples array
    let val: Value = if let Ok(mut file) = fs::File::open(path) {
        let mut buf = [0u8; 4096];
        let n = file.read(&mut buf).unwrap_or(0);
        if n > 0 {
            if let Some(pos) = buf[..n].windows(10).position(|w| w == b"\"samples\":") {
                let mut header_str = String::from_utf8_lossy(&buf[..pos]).to_string();
                header_str.push_str("\"samples\":[]}");
                serde_json::from_str(&header_str).ok()
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    }
    .or_else(|| {
        let data_str = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data_str).ok()
    })?;

    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let path_str = path.to_string_lossy().to_string();
    let date = val
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let time = val
        .get("time")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dur_secs = val
        .get("duration_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let duration = val
        .get("duration_label")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format_duration(dur_secs));
    let sample_count = val
        .get("sample_count")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(0);
    let metric_focus = val
        .get("metric_focus")
        .and_then(|v| v.as_str())
        .unwrap_or("ALL")
        .to_string();

    Some(RecordingMetadata {
        id,
        filename,
        path: path_str,
        date,
        time,
        duration,
        duration_seconds: dur_secs,
        sample_count,
        metric_focus,
    })
}

pub fn get_recordings_list() -> Vec<RecordingMetadata> {
    let (rec_dir, max_recs) = get_config();
    prune_recordings(&rec_dir, max_recs);

    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&rec_dir) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| {
                            s.starts_with("rec-")
                                && s.ends_with(".json")
                                && !s.ends_with(".timeline.json")
                        })
                        .unwrap_or(false)
            })
            .collect();

        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for f in files.into_iter().take(max_recs) {
            if let Some(meta) = read_recording_metadata(&f) {
                out.push(meta);
            }
        }
    }
    out
}

pub fn list_recordings() -> io::Result<()> {
    let out = get_recordings_list();
    let json_str = serde_json::to_string(&out).map_err(io::Error::other)?;
    println!("{}", json_str);
    Ok(())
}

pub fn save_session_data(
    samples: Value,
    dur_secs: u64,
    metric_focus: &str,
) -> io::Result<RecordingMetadata> {
    let (rec_dir, max_recs) = get_config();
    let (date_str, time_str, id_str) = local_datetime_strings();
    let filename = format!("{}.json", id_str);
    let filepath = rec_dir.join(&filename);

    let sample_count = samples.as_array().map(|a| a.len()).unwrap_or(0);
    let duration_label = format_duration(dur_secs);

    let record_obj = RecordingPayload {
        id: id_str.clone(),
        filename: filename.clone(),
        date: date_str.clone(),
        time: time_str.clone(),
        duration_seconds: dur_secs,
        duration_label: duration_label.clone(),
        metric_focus: metric_focus.to_string(),
        sample_count,
        samples,
    };

    let file_str = serde_json::to_string(&record_obj).map_err(io::Error::other)?;
    fs::write(&filepath, file_str)?;

    prune_recordings(&rec_dir, max_recs);

    Ok(RecordingMetadata {
        id: id_str,
        filename,
        path: filepath.to_string_lossy().to_string(),
        date: date_str,
        time: time_str,
        duration: duration_label,
        duration_seconds: dur_secs,
        sample_count,
        metric_focus: metric_focus.to_string(),
    })
}

pub fn save_recording(arg: Option<&str>) -> io::Result<()> {
    let raw = match arg {
        Some(path_str) => {
            let p = if let Some(rest) = path_str.strip_prefix("~/") {
                get_home_dir().join(rest)
            } else {
                PathBuf::from(path_str)
            };
            if p.exists() {
                fs::read_to_string(&p)?
            } else {
                path_str.to_string()
            }
        }
        None => {
            let mut buf = String::new();
            let n = io::stdin().read_line(&mut buf)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Empty input on stdin",
                ));
            }
            let trimmed = buf.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                buf
            } else {
                let _ = io::stdin().read_to_string(&mut buf);
                buf
            }
        }
    };

    let payload: Value = serde_json::from_str(&raw)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid JSON: {}", e)))?;

    let samples = payload
        .get("samples")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let sample_count = samples.as_array().map(|a| a.len()).unwrap_or(0);
    let dur_secs = payload
        .get("duration_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(sample_count as u64);
    let metric_focus = payload
        .get("metric_focus")
        .and_then(|v| v.as_str())
        .unwrap_or("ALL");

    let meta = save_session_data(samples, dur_secs, metric_focus)?;

    let resp = serde_json::json!({
        "status": "ok",
        "id": meta.id,
        "filename": meta.filename,
        "path": meta.path,
        "duration": meta.duration,
        "date": meta.date,
        "time": meta.time,
        "sample_count": meta.sample_count
    });

    println!(
        "{}",
        serde_json::to_string(&resp).map_err(io::Error::other)?
    );
    Ok(())
}

pub fn get_recording(target: Option<&str>) -> io::Result<()> {
    let target = target.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing recording ID or file path",
        )
    })?;
    let (rec_dir, _) = get_config();

    let path = recording_path_for(target, &rec_dir)?;

    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Recording file not found: {:?}", path),
        ));
    }

    let content = fs::read_to_string(&path)?;
    println!("{}", content.trim());
    Ok(())
}

pub fn load_recording_payload(target: &str) -> io::Result<RecordingPayload> {
    let (rec_dir, _) = get_config();
    let path = recording_path_for(target, &rec_dir)?;

    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Recording file not found: {:?}", path),
        ));
    }

    let content = fs::read_to_string(&path)?;
    let payload: RecordingPayload = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid JSON: {}", e)))?;
    Ok(payload)
}

pub fn delete_recording_by_id(target: &str) -> io::Result<bool> {
    let (rec_dir, _) = get_config();
    let path = recording_path_for(target, &rec_dir)?;

    // Deleting is destructive and reachable from the CLI, so it stays inside
    // the recordings directory even when the caller passed an explicit path.
    if !is_inside(&path, &rec_dir) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Refusing to delete outside the recordings directory: {path:?}"),
        ));
    }

    if path.exists() {
        let _ = fs::remove_file(&path);
        let mut p = path.clone();
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rec")
            .to_string();
        p.set_file_name(format!("{}.timeline.json", stem));
        let _ = fs::remove_file(&p);
        p.set_file_name(format!("{}.offsets.bin", stem));
        let _ = fs::remove_file(&p);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn delete_recording(target: Option<&str>) -> io::Result<()> {
    let target = target.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing recording ID or file path",
        )
    })?;

    if delete_recording_by_id(target)? {
        println!("{}", serde_json::json!({"status": "ok", "deleted": target}));
    } else {
        println!(
            "{}",
            serde_json::json!({"status": "not_found", "target": target})
        );
    }
    Ok(())
}

pub fn resolve_recording_path(target: &str, rec_dir: &Path) -> io::Result<PathBuf> {
    let path = recording_path_for(target, rec_dir)?;

    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Recording file not found: {:?}", path),
        ));
    }
    Ok(path)
}

pub fn get_timeline(target: &str) -> io::Result<()> {
    let (rec_dir, _) = get_config();
    let path = resolve_recording_path(target, &rec_dir)?;

    let cache_path = {
        let mut p = path.clone();
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("rec");
        p.set_file_name(format!("{}.timeline.json", stem));
        p
    };

    if cache_path.exists() {
        if let (Ok(orig_meta), Ok(cache_meta)) = (path.metadata(), cache_path.metadata()) {
            if let (Ok(orig_mtime), Ok(cache_mtime)) = (orig_meta.modified(), cache_meta.modified())
            {
                if cache_mtime >= orig_mtime {
                    if let Ok(cached) = fs::read_to_string(&cache_path) {
                        println!("{}", cached);
                        return Ok(());
                    }
                }
            }
        }
    }

    let content = fs::read_to_string(&path)?;
    let val: Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let filename = val
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let date = val
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let time = val
        .get("time")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dur_secs = val
        .get("duration_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let dur_label = val
        .get("duration_label")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| format_duration(dur_secs));
    let metric_focus = val
        .get("metric_focus")
        .and_then(|v| v.as_str())
        .unwrap_or("ALL")
        .to_string();

    let raw_samples = val.get("samples").and_then(|v| v.as_array());
    let mut timeline_samples = Vec::new();

    if let Some(samples) = raw_samples {
        for s in samples {
            let mut top_p = String::new();
            if let Some(procs) = s.get("processes").and_then(|v| v.as_array()) {
                if let Some(first) = procs.first() {
                    top_p = first
                        .get("name")
                        .or_else(|| first.get("cmd"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
            timeline_samples.push(serde_json::json!({
                "timestamp": s.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""),
                "cpu": s.get("cpu").and_then(|v| v.as_i64()).unwrap_or(0),
                "mem": s.get("mem").and_then(|v| v.as_i64()).unwrap_or(0),
                "io_mb": s.get("io_mb").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "gpu": s.get("gpu").and_then(|v| v.as_i64()).unwrap_or(0),
                "read_bps": s.get("read_bps").and_then(|v| v.as_u64()).unwrap_or(0),
                "write_bps": s.get("write_bps").and_then(|v| v.as_u64()).unwrap_or(0),
                "net_rx_bps": s.get("net_rx_bps").and_then(|v| v.as_u64()).unwrap_or(0),
                "net_tx_bps": s.get("net_tx_bps").and_then(|v| v.as_u64()).unwrap_or(0),
                "net_rate": s.get("net_rate").and_then(|v| v.as_u64()).unwrap_or(0),
                "top_process": top_p,
            }));
        }
    }

    let out = serde_json::json!({
        "id": id,
        "filename": filename,
        "path": path.to_string_lossy().to_string(),
        "date": date,
        "time": time,
        "duration_seconds": dur_secs,
        "duration_label": dur_label,
        "metric_focus": metric_focus,
        "sample_count": timeline_samples.len(),
        "samples": timeline_samples,
    });

    let out_str = serde_json::to_string(&out).map_err(io::Error::other)?;
    let _ = fs::write(&cache_path, &out_str);
    let _ = load_offsets(&path);
    println!("{}", out_str);
    Ok(())
}

pub fn scan_sample_offsets(bytes: &[u8]) -> Vec<(u64, u64)> {
    let mut offsets = Vec::new();
    let marker = b"\"samples\"";
    let mut marker_pos = None;
    for (i, w) in bytes.windows(marker.len()).enumerate() {
        if w == marker {
            marker_pos = Some(i + marker.len());
            break;
        }
    }
    let after_marker = match marker_pos {
        Some(pos) => pos,
        None => return offsets,
    };

    let mut samples_start = None;
    for (i, &b) in bytes[after_marker..].iter().enumerate() {
        if b == b'[' {
            samples_start = Some(after_marker + i + 1);
            break;
        } else if !b.is_ascii_whitespace() && b != b':' {
            break;
        }
    }

    let search_start = match samples_start {
        Some(pos) => pos,
        None => return offsets,
    };

    let mut in_str = false;
    let mut escaped = false;
    let mut depth: usize = 0;
    let mut sample_start = 0u64;

    for (idx, &b) in bytes[search_start..].iter().enumerate() {
        let cur = (search_start + idx) as u64;
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_str {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_str = !in_str;
            continue;
        }
        if !in_str {
            if b == b'{' {
                if depth == 0 {
                    sample_start = cur;
                }
                depth += 1;
            } else if b == b'}' {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        offsets.push((sample_start, cur + 1));
                    }
                }
            } else if b == b']' && depth == 0 {
                break;
            }
        }
    }

    offsets
}

pub fn load_offsets(path: &Path) -> io::Result<Vec<(u64, u64)>> {
    let mut offsets_file = path.to_path_buf();
    let stem = offsets_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rec");
    offsets_file.set_file_name(format!("{}.offsets.bin", stem));

    if offsets_file.exists() {
        if let (Ok(m1), Ok(m2)) = (path.metadata(), offsets_file.metadata()) {
            if let (Ok(t1), Ok(t2)) = (m1.modified(), m2.modified()) {
                if t2 >= t1 {
                    let bytes = fs::read(&offsets_file)?;
                    let num_entries = bytes.len() / 16;
                    let mut out = Vec::with_capacity(num_entries);
                    for chunk in bytes.as_chunks::<16>().0 {
                        let start = u64::from_le_bytes(chunk[0..8].try_into().unwrap());
                        let end = u64::from_le_bytes(chunk[8..16].try_into().unwrap());
                        out.push((start, end));
                    }
                    if !out.is_empty() {
                        return Ok(out);
                    }
                }
            }
        }
    }

    let file_bytes = fs::read(path)?;
    let offsets = scan_sample_offsets(&file_bytes);
    if !offsets.is_empty() {
        let mut out_bytes = Vec::with_capacity(offsets.len() * 16);
        for &(s, e) in &offsets {
            out_bytes.extend_from_slice(&s.to_le_bytes());
            out_bytes.extend_from_slice(&e.to_le_bytes());
        }
        let _ = fs::write(&offsets_file, out_bytes);
    }
    Ok(offsets)
}

pub fn inspect_recording(target: &str, index: usize, window: usize) -> io::Result<()> {
    let (rec_dir, _) = get_config();
    let path = resolve_recording_path(target, &rec_dir)?;

    let offsets = load_offsets(&path).unwrap_or_default();
    let total = offsets.len();

    let win = window.max(1);
    let start = index.saturating_sub(win);
    let end = if total > 0 {
        (index + win).min(total - 1)
    } else {
        0
    };

    let slices: Vec<Value> = if total > 0 && start <= end && end < total {
        let start_byte = offsets[start].0;
        let end_byte = offsets[end].1;
        let len = (end_byte - start_byte) as usize;

        let mut f = fs::File::open(&path)?;
        f.seek(SeekFrom::Start(start_byte))?;
        let mut chunk = vec![0u8; len];
        f.read_exact(&mut chunk)?;

        let mut json_buf = Vec::with_capacity(len + 2);
        json_buf.push(b'[');
        json_buf.extend_from_slice(&chunk);
        json_buf.push(b']');

        match serde_json::from_slice::<Vec<Value>>(&json_buf) {
            Ok(parsed) => {
                let mut out_slices = Vec::with_capacity(parsed.len());
                for (i, s) in (start..=end).zip(parsed) {
                    let ts = s.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
                    let procs = s
                        .get("processes")
                        .cloned()
                        .unwrap_or_else(|| Value::Array(Vec::new()));
                    out_slices.push(serde_json::json!({
                        "index": i,
                        "timestamp": ts,
                        "processes": procs,
                    }));
                }
                out_slices
            }
            Err(_) => legacy_inspect_slices(&path, start, end)?,
        }
    } else {
        legacy_inspect_slices(&path, start, end)?
    };

    let actual_total = if total > 0 { total } else { slices.len() };
    let out = serde_json::json!({
        "target_index": index,
        "start_index": start,
        "end_index": end,
        "total_samples": actual_total,
        "slices": slices,
    });

    println!("{}", serde_json::to_string(&out).map_err(io::Error::other)?);
    Ok(())
}

fn legacy_inspect_slices(path: &Path, start: usize, end: usize) -> io::Result<Vec<Value>> {
    let content = fs::read_to_string(path)?;
    let val: Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let samples = val
        .get("samples")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Recording has no samples array")
        })?;

    let mut slices = Vec::new();
    for i in start..=end {
        if let Some(s) = samples.get(i) {
            let ts = s.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
            let procs = s
                .get("processes")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            slices.push(serde_json::json!({
                "index": i,
                "timestamp": ts,
                "processes": procs,
            }));
        }
    }
    Ok(slices)
}

pub fn dispatch(subcmd: &str, args: &[String]) -> io::Result<()> {
    match subcmd {
        "list" => list_recordings(),
        "save" => save_recording(args.first().map(|s| s.as_str())),
        "get" => get_recording(args.first().map(|s| s.as_str())),
        "delete" => delete_recording(args.first().map(|s| s.as_str())),
        "timeline" => {
            let target = args.first().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Missing recording path or ID")
            })?;
            get_timeline(target)
        }
        "inspect" => {
            let target = args.first().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Missing recording path or ID")
            })?;
            let idx = args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let win = args
                .get(2)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(15);
            inspect_recording(target, idx, win)
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unknown record subcommand: {}", other),
        )),
    }
}

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

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
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

    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "Perfo",
            "History Exported",
            &format!("Saved: ~/{}.{{txt,json}}", basename),
        ])
        .spawn();

    Ok((
        txt_path.to_string_lossy().to_string(),
        json_path.to_string_lossy().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_id_rejects_path_separators() {
        assert!(sanitize_recording_id("rec-20260910-120000").is_ok());
        assert!(sanitize_recording_id("../../etc/passwd").is_err());
        assert!(sanitize_recording_id("sub/dir").is_err());
        assert!(sanitize_recording_id("..").is_err());
        assert!(sanitize_recording_id(".").is_err());
        assert!(sanitize_recording_id("").is_err());
    }

    #[test]
    fn bare_id_always_lands_in_the_recordings_dir() {
        let rec_dir = Path::new("/var/lib/perfo/recordings");
        let path = recording_path_for("rec-1", rec_dir).expect("valid id");
        assert_eq!(path, rec_dir.join("rec-1.json"));
    }

    #[test]
    fn traversing_id_never_produces_a_path() {
        let rec_dir = Path::new("/var/lib/perfo/recordings");
        assert!(recording_path_for("../../../etc/shadow", rec_dir).is_err());
    }

    #[test]
    fn is_inside_rejects_paths_outside_the_directory() {
        let dir = std::env::temp_dir().join("perfo-is-inside-test");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("create test dirs");

        assert!(is_inside(&dir.join("rec-1.json"), &dir));
        // A sibling, a parent, and a subdirectory are all outside a flat store.
        assert!(!is_inside(&nested.join("rec-1.json"), &dir));
        assert!(!is_inside(Path::new("/etc/passwd"), &dir));
        assert!(!is_inside(&dir.join("../escaped.json"), &dir));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_refuses_targets_outside_the_recordings_dir() {
        let dir = std::env::temp_dir().join("perfo-delete-guard-test");
        fs::create_dir_all(&dir).expect("create test dir");
        let victim = dir.join("victim.json");
        fs::write(&victim, "{}").expect("write victim");

        // PERFO_RECORDINGS_DIR points elsewhere, so an absolute path naming the
        // victim must be refused rather than deleted.
        let rec_dir = std::env::temp_dir().join("perfo-delete-guard-store");
        fs::create_dir_all(&rec_dir).expect("create store");
        unsafe {
            std::env::set_var("PERFO_RECORDINGS_DIR", &rec_dir);
        }

        let err = delete_recording_by_id(victim.to_str().expect("utf-8"))
            .expect_err("deleting outside the store must fail");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(victim.exists(), "the file must survive a refused delete");

        unsafe {
            std::env::remove_var("PERFO_RECORDINGS_DIR");
        }
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&rec_dir);
    }

    #[test]
    fn default_recordings_dir_follows_xdg_data_home() {
        // Absolute XDG_DATA_HOME wins; a relative one is ignored per the spec.
        assert!(default_recordings_dir().ends_with("perfo/recordings"));
    }

    #[test]
    fn test_scan_sample_offsets_basic() {
        let json = br#"{"id":"test","samples":[{"cpu":10},{"cpu":20,"cmd":"test}"},{"cpu":30}]}"#;
        let offsets = scan_sample_offsets(json);
        assert_eq!(offsets.len(), 3);
        assert_eq!(
            &json[offsets[0].0 as usize..offsets[0].1 as usize],
            br#"{"cpu":10}"#
        );
        assert_eq!(
            &json[offsets[1].0 as usize..offsets[1].1 as usize],
            br#"{"cpu":20,"cmd":"test}"}"#
        );
        assert_eq!(
            &json[offsets[2].0 as usize..offsets[2].1 as usize],
            br#"{"cpu":30}"#
        );
    }

    #[test]
    fn test_scan_sample_offsets_empty() {
        let json = br#"{"id":"test","samples":[]}"#;
        let offsets = scan_sample_offsets(json);
        assert!(offsets.is_empty());
    }
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, Read};
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

pub fn load_profile_env() -> (Option<String>, Option<usize>) {
    let mut rec_dir = None;
    let mut max_recs = None;
    let profile_path = get_home_dir().join(".bash_profile");
    if let Ok(content) = fs::read_to_string(&profile_path) {
        for line in content.lines() {
            let mut line = line.trim();
            if let Some(rest) = line.strip_prefix("export ") {
                line = rest.trim();
            }
            if let Some(rest) = line.strip_prefix("PERFO_RECORDINGS_DIR=") {
                let v = rest.trim().trim_matches('"').trim_matches('\'');
                rec_dir = Some(v.to_string());
            } else if let Some(rest) = line.strip_prefix("PERFO_MAX_RECORDINGS=") {
                let v = rest.trim().trim_matches('"').trim_matches('\'');
                if let Ok(num) = v.parse::<usize>() {
                    max_recs = Some(num);
                }
            }
        }
    }
    (rec_dir, max_recs)
}

pub fn get_config() -> (PathBuf, usize) {
    let (profile_dir, profile_max) = load_profile_env();
    let raw_dir = std::env::var("PERFO_RECORDINGS_DIR")
        .ok()
        .or(profile_dir)
        .unwrap_or_else(|| "~/.local/share/perfo/recordings".to_string());

    let expanded_dir = if let Some(rest) = raw_dir.strip_prefix("~/") {
        get_home_dir().join(rest)
    } else if raw_dir == "~" {
        get_home_dir()
    } else {
        PathBuf::from(raw_dir)
    };

    let _ = fs::create_dir_all(&expanded_dir);

    let max_recs = std::env::var("PERFO_MAX_RECORDINGS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .or(profile_max)
        .unwrap_or(5)
        .max(1);

    (expanded_dir, max_recs)
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
                        .map(|s| s.starts_with("rec-") && s.ends_with(".json"))
                        .unwrap_or(false)
            })
            .collect();

        // Sort descending by name (rec-YYYYMMDD-HHMMSS.json) which is chronological
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        if files.len() > max_recs {
            for f in &files[max_recs..] {
                let _ = fs::remove_file(f);
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
                        .map(|s| s.starts_with("rec-") && s.ends_with(".json"))
                        .unwrap_or(false)
            })
            .collect();

        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for f in files.into_iter().take(max_recs) {
            if let Ok(data_str) = fs::read_to_string(&f) {
                if let Ok(val) = serde_json::from_str::<Value>(&data_str) {
                    let id = val
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let filename = f
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let path = f.to_string_lossy().to_string();
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
                        .or_else(|| {
                            val.get("samples")
                                .and_then(|v| v.as_array())
                                .map(|a| a.len())
                        })
                        .unwrap_or(0);
                    let metric_focus = val
                        .get("metric_focus")
                        .and_then(|v| v.as_str())
                        .unwrap_or("ALL")
                        .to_string();

                    out.push(RecordingMetadata {
                        id,
                        filename,
                        path,
                        date,
                        time,
                        duration,
                        duration_seconds: dur_secs,
                        sample_count,
                        metric_focus,
                    });
                }
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

    let path = if target.ends_with(".json") {
        if let Some(rest) = target.strip_prefix("~/") {
            get_home_dir().join(rest)
        } else if target.contains('/') {
            PathBuf::from(target)
        } else {
            rec_dir.join(target)
        }
    } else {
        rec_dir.join(format!("{}.json", target))
    };

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
    let path = if target.ends_with(".json") {
        if let Some(rest) = target.strip_prefix("~/") {
            get_home_dir().join(rest)
        } else if target.contains('/') {
            PathBuf::from(target)
        } else {
            rec_dir.join(target)
        }
    } else {
        rec_dir.join(format!("{}.json", target))
    };

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
    let path = if target.ends_with(".json") {
        if target.contains('/') {
            PathBuf::from(target)
        } else {
            rec_dir.join(target)
        }
    } else {
        rec_dir.join(format!("{}.json", target))
    };

    if path.exists() {
        fs::remove_file(&path)?;
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

pub fn dispatch(subcmd: &str, arg: Option<&str>) -> io::Result<()> {
    match subcmd {
        "list" => list_recordings(),
        "save" => save_recording(arg),
        "get" => get_recording(arg),
        "delete" => delete_recording(arg),
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

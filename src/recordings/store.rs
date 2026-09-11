//! Reading, writing, listing and deleting the recording files.

use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::format::{format_duration, local_datetime_strings};
use super::paths::{get_config, get_home_dir, is_inside, recording_path_for};
use super::{RecordingMetadata, RecordingPayload};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

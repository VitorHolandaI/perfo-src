//! The byte-offset index that makes scrubbing a long recording O(1).

use serde_json::Value;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::format::format_duration;
use super::paths::{get_config, resolve_recording_path};

// qual:allow(test_quality, untested) reason: "prints a timeline built from load_offsets, which is tested"
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

// qual:allow(test_quality, untested) reason: "prints slices from load_offsets and legacy_inspect_slices, both tested"
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    /// A recording plus its sidecar, in a directory of its own so the tests
    /// stay independent.
    fn recording_fixture(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("perfo-timeline-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create fixture dir");
        let rec = dir.join("rec.json");
        fs::write(&rec, body).expect("write recording");
        (rec, dir.join("rec.offsets.bin"))
    }

    #[test]
    fn load_offsets_writes_a_sidecar_it_can_read_back() {
        let (rec, sidecar) = recording_fixture(
            "sidecar",
            r#"{"id":"t","samples":[{"cpu":10},{"cpu":20},{"cpu":30}]}"#,
        );

        let first = load_offsets(&rec).expect("scan the recording");
        assert_eq!(first.len(), 3);
        assert!(sidecar.exists(), "the scan must cache its offsets");

        // Second call takes the sidecar path and must agree with the scan.
        let second = load_offsets(&rec).expect("read the sidecar");
        assert_eq!(first, second);
    }

    #[test]
    fn load_offsets_ignores_a_sidecar_older_than_the_recording() {
        let (rec, sidecar) = recording_fixture("stale", r#"{"id":"t","samples":[{"cpu":1}]}"#);

        // A sidecar that predates the recording describes a file that has since
        // changed, so its entries would point at the wrong bytes.
        fs::write(&sidecar, vec![0u8; 16]).expect("write stale sidecar");
        let stale_time = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        filetime_set(&sidecar, stale_time);

        let offsets = load_offsets(&rec).expect("rescan the recording");
        assert_eq!(offsets.len(), 1);
        let bytes = fs::read(&rec).expect("read recording");
        assert_eq!(
            &bytes[offsets[0].0 as usize..offsets[0].1 as usize],
            br#"{"cpu":1}"#
        );
    }

    /// Backdates a file's mtime. std has no setter, so go through utimensat.
    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        let secs = when
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs() as i64;
        let times = [
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
        ];
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .expect("path has no interior nul");
        // SAFETY: c_path is a valid nul-terminated path and times is a 2-element
        // timespec array, which is exactly what utimensat documents.
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
        assert_eq!(rc, 0, "utimensat failed on {path:?}");
    }

    #[test]
    fn legacy_slices_carry_the_index_and_timestamp_of_each_sample() {
        let (rec, _) = recording_fixture(
            "legacy",
            r#"{"samples":[
                {"timestamp":"t0","processes":[{"pid":1}]},
                {"timestamp":"t1","processes":[]},
                {"timestamp":"t2"}
            ]}"#,
        );

        let slices = legacy_inspect_slices(&rec, 1, 5).expect("slice the recording");
        // The range is clamped by what exists, not padded to the requested end.
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0]["index"], 1);
        assert_eq!(slices[0]["timestamp"], "t1");
        // A sample with no process list still yields an empty array, never null.
        assert_eq!(slices[1]["processes"], serde_json::json!([]));
    }

    #[test]
    fn legacy_slices_reject_a_recording_with_no_samples_array() {
        let (rec, _) = recording_fixture("nosamples", r#"{"id":"t"}"#);

        let err = legacy_inspect_slices(&rec, 0, 1).expect_err("missing samples must fail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}

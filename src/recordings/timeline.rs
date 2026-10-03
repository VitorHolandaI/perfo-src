//! The byte-offset index that makes scrubbing a long recording O(1).

use serde_json::Value;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::format::format_duration;
use super::paths::{get_config, resolve_recording_path};

/// The `.timeline.json` sidecar that sits next to a recording.
fn timeline_cache_path(path: &Path) -> PathBuf {
    let mut cache = path.to_path_buf();
    let stem = cache.file_stem().and_then(|s| s.to_str()).unwrap_or("rec");
    cache.set_file_name(format!("{}.timeline.json", stem));
    cache
}

/// True while `sidecar` is at least as new as the file it derives from.
///
/// An older sidecar describes content that has since changed, and a missing or
/// unreadable one is treated the same way: rebuild it.
fn sidecar_is_current(source: &Path, sidecar: &Path) -> bool {
    let (Ok(source_time), Ok(sidecar_time)) = (
        source.metadata().and_then(|m| m.modified()),
        sidecar.metadata().and_then(|m| m.modified()),
    ) else {
        return false;
    };
    sidecar_time >= source_time
}

/// The cached timeline, but only while its sidecar is still current.
fn fresh_cached_timeline(path: &Path, cache_path: &Path) -> Option<String> {
    if !sidecar_is_current(path, cache_path) {
        return None;
    }
    fs::read_to_string(cache_path).ok()
}

fn str_field(val: &Value, key: &str, fallback: &str) -> String {
    val.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(fallback)
        .to_string()
}

/// One recorded sample reduced to the series the timeline draws, plus the name
/// of whichever process led that sample.
fn timeline_sample(sample: &Value) -> Value {
    let top_process = sample
        .get("processes")
        .and_then(|v| v.as_array())
        .and_then(|procs| procs.first())
        .map(|first| {
            first
                .get("name")
                .or_else(|| first.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();

    serde_json::json!({
        "timestamp": str_field(sample, "timestamp", ""),
        "cpu": sample.get("cpu").and_then(|v| v.as_i64()).unwrap_or(0),
        "mem": sample.get("mem").and_then(|v| v.as_i64()).unwrap_or(0),
        "io_mb": sample.get("io_mb").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "gpu": sample.get("gpu").and_then(|v| v.as_i64()).unwrap_or(0),
        "read_bps": sample.get("read_bps").and_then(|v| v.as_u64()).unwrap_or(0),
        "write_bps": sample.get("write_bps").and_then(|v| v.as_u64()).unwrap_or(0),
        "net_rx_bps": sample.get("net_rx_bps").and_then(|v| v.as_u64()).unwrap_or(0),
        "net_tx_bps": sample.get("net_tx_bps").and_then(|v| v.as_u64()).unwrap_or(0),
        "net_rate": sample.get("net_rate").and_then(|v| v.as_u64()).unwrap_or(0),
        "top_process": top_process,
    })
}

/// The recording's header carried over verbatim, with its samples replaced by
/// the reduced series.
fn timeline_document(path: &Path, val: &Value) -> Value {
    let dur_secs = val
        .get("duration_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let samples: Vec<Value> = val
        .get("samples")
        .and_then(|v| v.as_array())
        .map(|list| list.iter().map(timeline_sample).collect())
        .unwrap_or_default();

    serde_json::json!({
        "id": str_field(val, "id", ""),
        "filename": str_field(val, "filename", ""),
        "path": path.to_string_lossy().to_string(),
        "date": str_field(val, "date", ""),
        "time": str_field(val, "time", ""),
        "duration_seconds": dur_secs,
        "duration_label": val
            .get("duration_label")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format_duration(dur_secs)),
        "metric_focus": str_field(val, "metric_focus", "ALL"),
        "sample_count": samples.len(),
        "samples": samples,
    })
}

// qual:allow(test_quality, untested) reason: "prints a timeline built from load_offsets, which is tested"
pub fn get_timeline(target: &str) -> io::Result<()> {
    let (rec_dir, _) = get_config();
    let path = resolve_recording_path(target, &rec_dir)?;
    let cache_path = timeline_cache_path(&path);

    if let Some(cached) = fresh_cached_timeline(&path, &cache_path) {
        println!("{}", cached);
        return Ok(());
    }

    let content = fs::read_to_string(&path)?;
    let val: Value = serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let out_str =
        serde_json::to_string(&timeline_document(&path, &val)).map_err(io::Error::other)?;
    // Both sidecars are best-effort: a read-only store still serves timelines,
    // it just rebuilds them each time.
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

/// The `.offsets.bin` sidecar that indexes a recording's samples.
fn offsets_sidecar_path(path: &Path) -> PathBuf {
    let mut sidecar = path.to_path_buf();
    let stem = sidecar
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rec");
    sidecar.set_file_name(format!("{}.offsets.bin", stem));
    sidecar
}

/// Each entry is a little-endian `(start, end)` pair of u64 byte offsets, so
/// one sample costs 16 bytes and a trailing partial pair is ignored.
const OFFSET_ENTRY_BYTES: usize = 16;

/// Offsets read back from a sidecar that still describes this recording.
///
/// Returns `None` rather than an error on a damaged or unreadable sidecar: it
/// is a derived file, and rescanning always produces the same answer.
fn cached_offsets(path: &Path, sidecar: &Path) -> Option<Vec<(u64, u64)>> {
    if !sidecar_is_current(path, sidecar) {
        return None;
    }
    let bytes = fs::read(sidecar).ok()?;
    let out: Vec<(u64, u64)> = bytes
        .as_chunks::<OFFSET_ENTRY_BYTES>()
        .0
        .iter()
        .map(|chunk| {
            (
                u64::from_le_bytes(chunk[0..8].try_into().unwrap()),
                u64::from_le_bytes(chunk[8..16].try_into().unwrap()),
            )
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

pub fn load_offsets(path: &Path) -> io::Result<Vec<(u64, u64)>> {
    let sidecar = offsets_sidecar_path(path);
    if let Some(cached) = cached_offsets(path, &sidecar) {
        return Ok(cached);
    }

    let offsets = scan_sample_offsets(&fs::read(path)?);
    if !offsets.is_empty() {
        let mut bytes = Vec::with_capacity(offsets.len() * OFFSET_ENTRY_BYTES);
        for &(start, end) in &offsets {
            bytes.extend_from_slice(&start.to_le_bytes());
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        // Best-effort: a read-only store still scrubs, it just rescans.
        let _ = fs::write(&sidecar, bytes);
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
    fn timeline_sample_defaults_every_missing_series_to_zero() {
        let reduced = timeline_sample(&serde_json::json!({"timestamp": "t0"}));

        assert_eq!(reduced["timestamp"], "t0");
        // A sample recorded before a series existed must still draw as a gap
        // at zero rather than break the chart with a null.
        for key in ["cpu", "mem", "gpu", "read_bps", "net_rate"] {
            assert_eq!(reduced[key], 0, "{key} should default to 0");
        }
        assert_eq!(reduced["io_mb"], 0.0);
        assert_eq!(reduced["top_process"], "");
    }

    #[test]
    fn timeline_sample_names_the_leading_process_by_name_then_cmd() {
        let by_name = timeline_sample(&serde_json::json!({
            "processes": [{"name": "firefox", "cmd": "/usr/lib/firefox"}]
        }));
        assert_eq!(by_name["top_process"], "firefox");

        // Recorded samples predating the name field still carry cmd.
        let by_cmd = timeline_sample(&serde_json::json!({
            "processes": [{"cmd": "/usr/bin/cargo"}]
        }));
        assert_eq!(by_cmd["top_process"], "/usr/bin/cargo");

        let empty = timeline_sample(&serde_json::json!({"processes": []}));
        assert_eq!(empty["top_process"], "");
    }

    #[test]
    fn timeline_document_keeps_the_header_and_recounts_the_samples() {
        let recording = serde_json::json!({
            "id": "rec-1",
            "filename": "rec-1.json",
            "date": "2026-09-11",
            "time": "12:00",
            "duration_seconds": 90,
            "duration_label": "1m30s",
            "metric_focus": "CPU",
            "samples": [{"cpu": 10}, {"cpu": 20}],
        });

        let doc = timeline_document(Path::new("/store/rec-1.json"), &recording);

        assert_eq!(doc["id"], "rec-1");
        assert_eq!(doc["metric_focus"], "CPU");
        assert_eq!(doc["duration_label"], "1m30s");
        assert_eq!(doc["path"], "/store/rec-1.json");
        // sample_count is recomputed, never carried over from the recording.
        assert_eq!(doc["sample_count"], 2);
        assert_eq!(doc["samples"].as_array().expect("array").len(), 2);
    }

    #[test]
    fn timeline_document_falls_back_to_a_formatted_duration_and_all_focus() {
        let doc = timeline_document(
            Path::new("/store/rec-2.json"),
            &serde_json::json!({"duration_seconds": 90}),
        );

        assert_eq!(doc["duration_label"], format_duration(90));
        assert_eq!(doc["metric_focus"], "ALL");
        assert_eq!(doc["sample_count"], 0);
    }

    #[test]
    fn a_sidecar_older_than_its_source_is_not_current() {
        let (rec, _) = recording_fixture("sidecar-age", r#"{"samples":[]}"#);
        let cache = timeline_cache_path(&rec);
        assert!(cache.ends_with("rec.timeline.json"));

        // Missing counts as stale: there is nothing to serve.
        assert!(!sidecar_is_current(&rec, &cache));
        assert!(fresh_cached_timeline(&rec, &cache).is_none());

        fs::write(&cache, "{\"cached\":true}").expect("write cache");
        assert!(sidecar_is_current(&rec, &cache));
        assert_eq!(
            fresh_cached_timeline(&rec, &cache).as_deref(),
            Some("{\"cached\":true}")
        );

        filetime_set(
            &cache,
            std::time::SystemTime::now() - std::time::Duration::from_secs(60),
        );
        assert!(!sidecar_is_current(&rec, &cache));
    }

    #[test]
    fn str_field_falls_back_when_absent_or_not_a_string() {
        let val = serde_json::json!({"present": "yes", "numeric": 7});
        assert_eq!(str_field(&val, "present", "fb"), "yes");
        assert_eq!(str_field(&val, "absent", "fb"), "fb");
        assert_eq!(str_field(&val, "numeric", "fb"), "fb");
    }

    #[test]
    fn legacy_slices_reject_a_recording_with_no_samples_array() {
        let (rec, _) = recording_fixture("nosamples", r#"{"id":"t"}"#);

        let err = legacy_inspect_slices(&rec, 0, 1).expect_err("missing samples must fail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}

//! The byte-offset index that makes scrubbing a long recording O(1), and the
//! `record inspect` window built on it.
//!
//! The scalar reduction a chart draws lives in `timeline.rs`.

use serde_json::Value;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::paths::{get_config, resolve_recording_path};
use super::timeline::{process_list, sidecar_is_current};

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
                    let procs = process_list(&s)
                        .map(|p| Value::Array(p.clone()))
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
            let procs = process_list(s)
                .map(|p| Value::Array(p.clone()))
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
    use crate::recordings::test_support::{filetime_set, recording_fixture};

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

    #[test]
    fn legacy_slices_carry_the_process_list_under_either_key() {
        let (rec, _) = recording_fixture(
            "either-key",
            r#"{"samples":[
                {"timestamp":"t0","top_procs":[{"pid":1},{"pid":2}]},
                {"timestamp":"t1","processes":[{"pid":3}]}
            ]}"#,
        );

        let slices = legacy_inspect_slices(&rec, 0, 1).expect("slice the recording");
        assert_eq!(slices[0]["processes"].as_array().expect("array").len(), 2);
        assert_eq!(slices[1]["processes"].as_array().expect("array").len(), 1);
    }
}

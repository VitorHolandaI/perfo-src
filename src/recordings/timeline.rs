//! Reducing a recording to the scalar series a chart draws, and the
//! `.timeline.json` sidecar that caches the result.
//!
//! The byte index that makes scrubbing cheap lives in `offsets.rs`.

use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::format::format_duration;
use super::offsets::load_offsets;
use super::paths::{get_config, resolve_recording_path};

/// The `.timeline.json` sidecar that sits next to a recording.
fn timeline_cache_path(path: &Path) -> PathBuf {
    let mut cache = path.to_path_buf();
    let stem = cache.file_stem().and_then(|s| s.to_str()).unwrap_or("rec");
    cache.set_file_name(format!("{}.timeline.json", stem));
    cache
}

/// Bumped whenever the reduced series change shape, so a sidecar written by an
/// older build is rebuilt instead of served.
///
/// - v1 holds a flat zero for cpu, mem and gpu, and an empty `top_process`.
/// - v2 is the first that reads the float series and the `top_procs` key the
///   TUI writes.
/// - v3 adds the thermal series, which carry `null` for an absent sensor
///   rather than the zero every other series defaults to.
const TIMELINE_SCHEMA: u64 = 3;

/// True while `sidecar` was written by this build's reducer.
///
/// A sidecar from an older schema is stale however recent it is: the bad one
/// is always newer than the recording it derives from, so mtime alone would
/// serve it forever.
fn sidecar_schema_is_current(sidecar: &Path) -> bool {
    let Ok(text) = fs::read_to_string(sidecar) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("schema").and_then(|s| s.as_u64()))
        .is_some_and(|v| v == TIMELINE_SCHEMA)
}

/// True while `sidecar` is at least as new as the file it derives from.
///
/// An older sidecar describes content that has since changed, and a missing or
/// unreadable one is treated the same way: rebuild it.
pub(super) fn sidecar_is_current(source: &Path, sidecar: &Path) -> bool {
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
    if !sidecar_is_current(path, cache_path) || !sidecar_schema_is_current(cache_path) {
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

/// A percentage series, read through `as_f64` so it survives both producers:
/// the TUI writes `f32`, the widget writes a rounded integer, and `as_i64`
/// answered `None` to every float the TUI recorded.
///
/// Rounded to one decimal because the panel concatenates these values straight
/// into its status line, where full `f32` precision is noise.
fn percent_field(sample: &Value, key: &str) -> f64 {
    let raw = sample.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    if raw.is_finite() {
        (raw * 10.0).round() / 10.0
    } else {
        0.0
    }
}

/// A bytes-per-second series. Also read through `as_f64`, since a rate derived
/// from a delta is not always written as a whole number; a negative one is
/// clamped rather than wrapped to `u64::MAX`.
fn rate_field(sample: &Value, key: &str) -> u64 {
    let raw = sample.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
    if raw.is_finite() && raw > 0.0 {
        raw.round() as u64
    } else {
        0
    }
}

/// A thermal reading, kept as `null` when the sensor is absent. Zero degrees
/// is a real measurement, so defaulting to it the way the other series do
/// would invent a cold machine out of a missing sensor.
fn temp_field(sample: &Value, key: &str) -> Value {
    match sample.get(key).and_then(|v| v.as_f64()) {
        Some(raw) if raw.is_finite() => Value::from(raw.round() as i64),
        _ => Value::Null,
    }
}

/// The per-core temperatures, with the same hole-preserving rule.
fn per_core_temps(sample: &Value) -> Value {
    let Some(list) = sample.get("per_core_temp_c").and_then(|v| v.as_array()) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        list.iter()
            .map(|c| match c.as_f64() {
                Some(raw) if raw.is_finite() => Value::from(raw.round() as i64),
                _ => Value::Null,
            })
            .collect(),
    )
}

/// The recorded process list. `HistorySample` serialises it as `top_procs` and
/// takes `processes` only as a deserialisation alias, so both names reach this
/// code and reading just one of them loses the whole table.
pub(super) fn process_list(sample: &Value) -> Option<&Vec<Value>> {
    sample
        .get("top_procs")
        .or_else(|| sample.get("processes"))
        .and_then(|v| v.as_array())
}

/// One recorded sample reduced to the series the timeline draws, plus the name
/// of whichever process led that sample.
fn timeline_sample(sample: &Value) -> Value {
    let top_process = process_list(sample)
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
        "cpu": percent_field(sample, "cpu"),
        "mem": percent_field(sample, "mem"),
        "io_mb": sample.get("io_mb").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "gpu": percent_field(sample, "gpu"),
        "read_bps": rate_field(sample, "read_bps"),
        "write_bps": rate_field(sample, "write_bps"),
        "net_rx_bps": rate_field(sample, "net_rx_bps"),
        "net_tx_bps": rate_field(sample, "net_tx_bps"),
        "net_rate": rate_field(sample, "net_rate"),
        "cpu_temp_c": temp_field(sample, "cpu_temp_c"),
        "gpu_temp_c": temp_field(sample, "gpu_temp_c"),
        "disk_temp_c": temp_field(sample, "disk_temp_c"),
        "fan_rpm": temp_field(sample, "fan_rpm"),
        "per_core_temp_c": per_core_temps(sample),
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
        "schema": TIMELINE_SCHEMA,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recordings::test_support::{filetime_set, recording_fixture};

    #[test]
    fn timeline_sample_defaults_every_missing_series_to_zero() {
        let reduced = timeline_sample(&serde_json::json!({"timestamp": "t0"}));

        assert_eq!(reduced["timestamp"], "t0");
        // A sample recorded before a series existed must still draw as a gap
        // at zero rather than break the chart with a null.
        for key in [
            "read_bps",
            "write_bps",
            "net_rx_bps",
            "net_tx_bps",
            "net_rate",
        ] {
            assert_eq!(reduced[key], 0, "{key} should default to 0");
        }
        for key in ["cpu", "mem", "gpu", "io_mb"] {
            assert_eq!(reduced[key], 0.0, "{key} should default to 0.0");
        }
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

    /// Every recording made before this fix has a sidecar holding a flat zero
    /// for cpu, mem and gpu, and that sidecar is newer than the recording it
    /// came from, so mtime alone would serve the broken reduction forever.
    #[test]
    fn a_sidecar_from_an_older_schema_is_rebuilt_however_recent_it_is() {
        let (rec, _) = recording_fixture("sidecar-schema", r#"{"samples":[]}"#);
        let cache = timeline_cache_path(&rec);

        fs::write(&cache, "{\"id\":\"rec\",\"samples\":[]}").expect("write v1 cache");
        assert!(sidecar_is_current(&rec, &cache));
        assert!(!sidecar_schema_is_current(&cache));
        assert!(fresh_cached_timeline(&rec, &cache).is_none());

        let older = format!("{{\"schema\":{}}}", TIMELINE_SCHEMA - 1);
        fs::write(&cache, &older).expect("write older cache");
        assert!(!sidecar_schema_is_current(&cache));

        fs::write(&cache, "not json at all").expect("write junk");
        assert!(!sidecar_schema_is_current(&cache));
    }

    /// Temperature is the one series where a missing reading cannot default to
    /// zero: zero degrees is a real cold measurement, so the sidecar carries
    /// `null` and the chart draws a gap.
    #[test]
    fn an_absent_thermal_sensor_reduces_to_null_not_zero() {
        let reduced = timeline_sample(&serde_json::json!({
            "cpu_temp_c": 61,
            "gpu_temp_c": serde_json::Value::Null,
        }));

        assert_eq!(reduced["cpu_temp_c"], 61);
        assert!(reduced["gpu_temp_c"].is_null(), "no sensor stays null");
        // A sample recorded before thermal existed has no key at all.
        assert!(reduced["disk_temp_c"].is_null());
        assert!(reduced["fan_rpm"].is_null());
    }

    /// The TUI writes these as integers already, but a hand-edited or older
    /// recording can hold a float, and a temperature rounds rather than
    /// truncates.
    #[test]
    fn a_thermal_reading_written_as_a_float_rounds_to_whole_degrees() {
        let reduced = timeline_sample(&serde_json::json!({
            "cpu_temp_c": 61.6,
            "disk_temp_c": 33.85,
        }));

        assert_eq!(reduced["cpu_temp_c"], 62);
        assert_eq!(reduced["disk_temp_c"], 34);
    }

    #[test]
    fn per_core_temperatures_keep_their_holes_so_core_indices_still_line_up() {
        let reduced = timeline_sample(&serde_json::json!({
            "per_core_temp_c": [52, serde_json::Value::Null, 68.6],
        }));

        assert_eq!(
            reduced["per_core_temp_c"],
            serde_json::json!([52, null, 69])
        );
        // Absent entirely reduces to an empty list, never to null.
        let none = timeline_sample(&serde_json::json!({}));
        assert_eq!(none["per_core_temp_c"], serde_json::json!([]));
    }

    #[test]
    fn a_timeline_document_carries_the_schema_it_was_reduced_with() {
        let doc = timeline_document(Path::new("/store/rec.json"), &serde_json::json!({}));
        assert_eq!(doc["schema"], TIMELINE_SCHEMA);
    }

    #[test]
    fn a_sidecar_older_than_its_source_is_not_current() {
        let (rec, _) = recording_fixture("sidecar-age", r#"{"samples":[]}"#);
        let cache = timeline_cache_path(&rec);
        assert!(cache.ends_with("rec.timeline.json"));

        // Missing counts as stale: there is nothing to serve.
        assert!(!sidecar_is_current(&rec, &cache));
        assert!(fresh_cached_timeline(&rec, &cache).is_none());

        // Written against the constant, so a schema bump does not need this
        // test edited to keep passing.
        let fresh = format!("{{\"schema\":{TIMELINE_SCHEMA},\"cached\":true}}");
        fs::write(&cache, &fresh).expect("write cache");
        assert!(sidecar_is_current(&rec, &cache));
        assert_eq!(
            fresh_cached_timeline(&rec, &cache).as_deref(),
            Some(fresh.as_str())
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

    /// The TUI serialises `cpu`, `mem` and `gpu` from `f32`, so they reach the
    /// reducer as JSON floats. Reading them as integers silently dropped every
    /// one of them and the replay drew three flat lines at zero.
    #[test]
    fn timeline_sample_keeps_the_float_series_the_tui_records() {
        let reduced = timeline_sample(&serde_json::json!({
            "cpu": 1.2512030601501465_f64,
            "mem": 60.941959381103516_f64,
            "gpu": 37.5_f64,
            "io_mb": 3742.72_f64,
        }));

        // Rounded to one decimal: the panel concatenates these straight into
        // its status line, where full f32 precision is unreadable.
        assert_eq!(reduced["cpu"], 1.3);
        assert_eq!(reduced["mem"], 60.9);
        assert_eq!(reduced["gpu"], 37.5);
        assert_eq!(reduced["io_mb"], 3742.72);
    }

    /// The widget rounds its own samples before writing them, so the same
    /// series arrives as integers from that producer.
    #[test]
    fn timeline_sample_keeps_the_integer_series_the_widget_records() {
        let reduced = timeline_sample(&serde_json::json!({"cpu": 10, "mem": 61, "gpu": 0}));

        assert_eq!(reduced["cpu"], 10.0);
        assert_eq!(reduced["mem"], 61.0);
        assert_eq!(reduced["gpu"], 0.0);
    }

    /// `HistorySample` serialises the list as `top_procs` and only accepts
    /// `processes` as a deserialisation alias, so the reducer has to know both
    /// names or it never finds the leading process of a TUI recording.
    #[test]
    fn timeline_sample_names_the_leading_process_under_either_key() {
        let from_tui = timeline_sample(&serde_json::json!({
            "top_procs": [{"name": "cargo", "cmd": "/usr/bin/cargo"}]
        }));
        assert_eq!(from_tui["top_process"], "cargo");

        let from_widget = timeline_sample(&serde_json::json!({
            "processes": [{"name": "firefox"}]
        }));
        assert_eq!(from_widget["top_process"], "firefox");
    }

    /// A rate written as a float still has to land as a whole number of bytes
    /// per second rather than default to zero.
    #[test]
    fn timeline_sample_rounds_a_rate_written_as_a_float() {
        let reduced = timeline_sample(&serde_json::json!({
            "read_bps": 1024.7_f64,
            "net_rx_bps": -5.0_f64,
        }));

        assert_eq!(reduced["read_bps"], 1025);
        // A negative rate is not a thing; clamping beats wrapping to u64::MAX.
        assert_eq!(reduced["net_rx_bps"], 0);
    }
}

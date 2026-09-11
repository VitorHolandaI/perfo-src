//! GPU detection and utilisation across AMD, Intel and NVIDIA.

mod amd;
mod intel;
mod monitor;
mod sysfs;

pub use monitor::GpuMonitor;

use serde::Serialize;

const AMD_VENDOR_ID: &str = "0x1002";

const INTEL_VENDOR_ID: &str = "0x8086";

const DRM_ENGINE_COUNT: usize = 5;

#[derive(Clone, Serialize)]
pub struct GpuProcessInfo {
    pub pid: u32,
    pub gpu_percent: Option<f32>,
    pub memory_used_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub usage_percent: Option<f32>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub processes: Vec<GpuProcessInfo>,
}

#[derive(Clone, Default, Serialize)]
pub struct GpuSnapshot {
    pub devices: Vec<GpuInfo>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::intel::{
        engine_usage_percent, parse_drm_fdinfo, process_gpu_usage, IntelEngineTimes,
    };
    use super::sysfs::{hwmon_temperature, is_drm_card};

    #[test]
    fn drm_card_names_are_strict() {
        assert!(is_drm_card("card0"));
        assert!(is_drm_card("card12"));
        assert!(!is_drm_card("card"));
        assert!(!is_drm_card("card0-HDMI-A-1"));
    }

    #[test]
    fn percent_values_are_clamped() {
        assert_eq!(read_percent_from("55.5"), Some(55.5));
        assert_eq!(read_percent_from("150"), Some(100.0));
        assert_eq!(read_percent_from("bad"), None);
    }

    #[test]
    fn hwmon_uses_input_not_temperature_limits() {
        let base = std::env::temp_dir().join(format!("perfo-gpu-hwmon-{}", std::process::id()));
        let hwmon = base.join("hwmon/hwmon0");
        std::fs::create_dir_all(&hwmon).unwrap();
        std::fs::write(hwmon.join("temp1_input"), "45000\n").unwrap();
        std::fs::write(hwmon.join("temp1_max"), "100000\n").unwrap();
        std::fs::write(hwmon.join("temp1_crit"), "110000\n").unwrap();
        assert_eq!(hwmon_temperature(&base), Some(45.0));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn drm_fdinfo_reads_i915_engine_times() {
        let raw = "drm-client-id: 42\ndrm-pdev: 0000:00:02.0\ndrm-engine-render: 123 ns\ndrm-engine-copy: 9 ns\ndrm-engine-capacity-video: 2\n";
        let sample = parse_drm_fdinfo(raw, "0000:00:02.0").expect("i915 fdinfo");
        assert_eq!(sample.client_id, 42);
        assert_eq!(sample.engines.values, [123, 9, 0, 0, 0]);
    }

    #[test]
    fn drm_fdinfo_rejects_other_devices() {
        let raw = "drm-client-id: 42\ndrm-pdev: 0000:00:03.0\n";
        assert!(parse_drm_fdinfo(raw, "0000:00:02.0").is_none());
    }

    #[test]
    fn engine_usage_uses_the_busiest_engine() {
        let previous = IntelEngineTimes {
            values: [100, 200, 300, 400, 500],
        };
        let current = IntelEngineTimes {
            values: [200, 300, 400, 500, 600],
        };
        assert_eq!(engine_usage_percent(previous, current, 1.0), Some(0.00001));
    }

    #[test]
    fn process_gpu_usage_tracks_current_pids() {
        let previous = HashMap::from([(10, IntelEngineTimes { values: [0; 5] })]);
        let current = HashMap::from([
            (
                10,
                IntelEngineTimes {
                    values: [500_000_000, 0, 0, 0, 0],
                },
            ),
            (
                20,
                IntelEngineTimes {
                    values: [1, 0, 0, 0, 0],
                },
            ),
        ]);
        let processes = process_gpu_usage(&previous, &current, 1.0);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].pid, 10);
        assert_eq!(processes[0].gpu_percent, Some(50.0));
    }

    fn read_percent_from(raw: &str) -> Option<f32> {
        let value = raw.trim().parse::<f32>().ok()?;
        value.is_finite().then(|| value.clamp(0.0, 100.0))
    }
}

//! AMD GPUs, read straight from the amdgpu sysfs attributes.

use std::fs;
use std::path::PathBuf;

use super::sysfs::{hwmon_temperature, is_drm_card};
use super::sysfs::{read_percent, read_trimmed, read_u64};
use super::{GpuInfo, AMD_VENDOR_ID};

pub(super) struct AmdDevice {
    path: PathBuf,
    name: String,
}

impl AmdDevice {
    pub(super) fn discover() -> Vec<Self> {
        let Ok(entries) = fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        let mut devices: Vec<Self> = entries
            .filter_map(Result::ok)
            .filter(|entry| is_drm_card(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| {
                let path = entry.path().join("device");
                if read_trimmed(&path.join("vendor")).as_deref() != Some(AMD_VENDOR_ID) {
                    return None;
                }
                let name = read_trimmed(&path.join("product_name"))
                    .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
                Some(Self { path, name })
            })
            .collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }

    pub(super) fn snapshot(&self) -> GpuInfo {
        GpuInfo {
            name: self.name.clone(),
            vendor: "AMD".into(),
            usage_percent: read_percent(&self.path.join("gpu_busy_percent")),
            memory_used_bytes: read_u64(&self.path.join("mem_info_vram_used")),
            memory_total_bytes: read_u64(&self.path.join("mem_info_vram_total")),
            temperature_c: hwmon_temperature(&self.path),
            power_w: read_u64(&self.path.join("power1_average")).map(|u| u as f32 / 1_000_000.0),
            processes: Vec::new(),
        }
    }
}

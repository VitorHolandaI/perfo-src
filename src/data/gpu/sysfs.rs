//! Small readers for the sysfs attribute files the GPU backends share.

use std::fs;
use std::path::Path;

pub(super) fn is_drm_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
    })
}

pub(super) fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().into())
}

pub(super) fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

pub(super) fn read_percent(path: &Path) -> Option<f32> {
    let value = read_trimmed(path)?.parse::<f32>().ok()?;
    value
        .is_finite()
        .then(|| crate::units::clamp_percent(value))
}

pub(super) fn hwmon_temperature(device: &Path) -> Option<f32> {
    let hwmon = device.join("hwmon");
    let entries = fs::read_dir(hwmon).ok()?;
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| fs::read_dir(entry.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("temp") && name.ends_with("_input")
        })
        .filter_map(|entry| read_u64(&entry.path()))
        .map(|millidegrees| millidegrees as f32 / 1000.0)
        .max_by(|a, b| a.total_cmp(b))
}

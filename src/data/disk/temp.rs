//! Drive temperature, read from hwmon.

use std::path::Path;

use super::parse::MS_PER_SEC;

/// First hwmon temp1_input (millidegrees) under `dir`, preferring the NVMe
/// controller (hwmon "name" == "nvme"); other sensors are a fallback since
/// some block devices expose unrelated hwmon temps.
pub(super) fn scan_hwmon(dir: &Path) -> Option<f32> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback: Option<f32> = None;
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with("hwmon") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(e.path().join("temp1_input")) else {
            continue;
        };
        let Ok(milli) = raw.trim().parse::<i64>() else {
            continue;
        };
        let is_nvme = std::fs::read_to_string(e.path().join("name"))
            .map(|n| n.trim() == "nvme")
            .unwrap_or(false);
        if is_nvme {
            return Some(milli as f32 / MS_PER_SEC);
        }
        if fallback.is_none() {
            fallback = Some(milli as f32 / MS_PER_SEC);
        }
    }
    fallback
}

/// Device temperature in °C from the first hwmon temp1_input of the disk
/// node. Candidates in order: the node itself, its `device` symlink (which
/// on sysfs points at the controller that owns hwmon), the parent disk node
/// for partitions, and each dm slave plus its parent.
pub(super) fn nvme_temp_c(block_dir: &Path) -> Option<f32> {
    let name = block_dir.file_name()?.to_str()?;
    let mut nodes: Vec<std::path::PathBuf> =
        vec![block_dir.to_path_buf(), block_dir.join("device")];
    if !block_dir.join("device").is_dir() {
        if let Some(p) = name.rfind('p') {
            nodes.push(block_dir.parent()?.join(&name[..p]));
        } else {
            for slave in std::fs::read_dir(block_dir.join("slaves")).ok()?.flatten() {
                // slaves/ entries are symlinks to the real disk node.
                let real = std::fs::canonicalize(slave.path()).ok()?;
                nodes.push(real.clone());
                if let Some(parent) = real.parent() {
                    nodes.push(parent.to_path_buf());
                }
            }
        }
    }
    for node in nodes {
        if let Some(t) = scan_hwmon(&node.join("device")).or_else(|| scan_hwmon(&node)) {
            return Some(t);
        }
    }
    None
}

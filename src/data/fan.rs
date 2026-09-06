use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct FanInfo {
    pub id: String,
    pub chip: String,
    pub label: String,
    pub rpm: u64,
}

#[derive(Clone, Default, Serialize)]
pub struct FanSnapshot {
    pub fans: Vec<FanInfo>,
}

pub struct FanMonitor {
    root: PathBuf,
}

impl Default for FanMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl FanMonitor {
    pub fn new() -> Self {
        Self::with_root("/sys/class/hwmon")
    }

    fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn snapshot(&self) -> FanSnapshot {
        let mut fans = discover_fans(&self.root);
        deduplicate_known_views(&mut fans);
        FanSnapshot { fans }
    }
}

fn discover_fans(root: &Path) -> Vec<FanInfo> {
    let Ok(chips) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut fans = chips
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("hwmon"))
        .flat_map(|entry| fans_from_chip(&entry.path()))
        .collect::<Vec<_>>();
    fans.sort_by(|a, b| a.id.cmp(&b.id));
    fans
}

fn fans_from_chip(chip_path: &Path) -> Vec<FanInfo> {
    let chip = read_trimmed(&chip_path.join("name")).unwrap_or_default();
    if chip.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(chip_path) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let index = fan_index(&entry.file_name().to_string_lossy())?;
            let rpm = read_trimmed(&entry.path())?.parse().ok()?;
            let label = read_trimmed(&chip_path.join(format!("fan{index}_label")))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("Fan {index}"));
            Some(FanInfo {
                id: format!("{chip}/fan{index}"),
                chip: chip.clone(),
                label: clean_label(&label),
                rpm,
            })
        })
        .collect()
}

fn deduplicate_known_views(fans: &mut Vec<FanInfo>) {
    let embedded_controller = fans.iter().any(|fan| fan.chip == "cros_ec");
    if embedded_controller {
        fans.retain(|fan| fan.chip != "acpi_fan" && fan.chip != "asus");
    } else if fans.iter().any(|fan| fan.chip == "acpi_fan") {
        fans.retain(|fan| fan.chip != "asus");
    }
}

fn fan_index(name: &str) -> Option<u32> {
    name.strip_prefix("fan")?
        .strip_suffix("_input")?
        .parse()
        .ok()
}

fn clean_label(label: &str) -> String {
    let clean = label
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    clean.trim().chars().take(64).collect()
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_index_accepts_only_numeric_inputs() {
        assert_eq!(fan_index("fan1_input"), Some(1));
        assert_eq!(fan_index("fan12_input"), Some(12));
        assert_eq!(fan_index("fan_input"), None);
        assert_eq!(fan_index("fan1_label"), None);
    }

    #[test]
    fn clean_label_removes_controls_and_caps_length() {
        let label = clean_label(" CPU\tFan\n".to_owned().as_str());
        assert_eq!(label, "CPU Fan");
        assert_eq!(clean_label(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn cros_ec_replaces_acpi_view() {
        let mut fans = vec![
            FanInfo {
                id: "acpi_fan/fan1".into(),
                chip: "acpi_fan".into(),
                label: "ACPI".into(),
                rpm: 1200,
            },
            FanInfo {
                id: "cros_ec/fan1".into(),
                chip: "cros_ec".into(),
                label: "EC".into(),
                rpm: 1200,
            },
        ];
        deduplicate_known_views(&mut fans);
        assert_eq!(fans.len(), 1);
        assert_eq!(fans[0].chip, "cros_ec");
    }

    #[test]
    fn acpi_replaces_unstable_asus_view() {
        let mut fans = vec![
            FanInfo {
                id: "acpi_fan/fan1".into(),
                chip: "acpi_fan".into(),
                label: "ACPI".into(),
                rpm: 3500,
            },
            FanInfo {
                id: "asus/fan1".into(),
                chip: "asus".into(),
                label: "cpu_fan".into(),
                rpm: 24000,
            },
        ];
        deduplicate_known_views(&mut fans);
        assert_eq!(fans.len(), 1);
        assert_eq!(fans[0].chip, "acpi_fan");
    }
}

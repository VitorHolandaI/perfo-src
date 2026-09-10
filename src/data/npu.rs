use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

const INTEL_VENDOR_ID: &str = "0x8086";
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Serialize)]
pub struct NpuInfo {
    pub name: String,
    pub pci_address: String,
    pub vendor_id: String,
    pub device_id: String,
    pub utilization_percent: Option<f32>,
    pub current_frequency_mhz: Option<u64>,
    pub max_frequency_mhz: Option<u64>,
    pub memory_used_bytes: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
pub struct NpuSnapshot {
    pub devices: Vec<NpuInfo>,
}

struct NpuDevice {
    path: PathBuf,
    pci_address: String,
    vendor_id: String,
    device_id: String,
    previous_busy: Option<(u64, Instant)>,
    utilization_percent: Option<f32>,
    current_frequency_mhz: Option<u64>,
    max_frequency_mhz: Option<u64>,
    memory_used_bytes: Option<u64>,
}

impl NpuDevice {
    fn refresh_at(&mut self, now: Instant) {
        if let Some(current_busy) = read_u64(&self.path.join("npu_busy_time_us")) {
            self.utilization_percent = self
                .previous_busy
                .and_then(|previous| utilization_percent(previous, (current_busy, now)));
            self.previous_busy = Some((current_busy, now));
        } else {
            self.utilization_percent = None;
        }
        self.current_frequency_mhz = read_u64(&self.path.join("npu_current_frequency_mhz"));
        self.max_frequency_mhz = read_u64(&self.path.join("npu_max_frequency_mhz"));
        self.memory_used_bytes = read_u64(&self.path.join("npu_memory_utilization"));
    }

    fn snapshot(&self) -> NpuInfo {
        NpuInfo {
            name: "Intel NPU".into(),
            pci_address: self.pci_address.clone(),
            vendor_id: self.vendor_id.clone(),
            device_id: self.device_id.clone(),
            utilization_percent: self.utilization_percent,
            current_frequency_mhz: self.current_frequency_mhz,
            max_frequency_mhz: self.max_frequency_mhz,
            memory_used_bytes: self.memory_used_bytes,
        }
    }
}

pub struct NpuMonitor {
    devices: Vec<NpuDevice>,
    last_refresh: Option<Instant>,
}

impl Default for NpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl NpuMonitor {
    pub fn new() -> Self {
        Self::with_root(Path::new("/sys/class/accel"))
    }

    pub fn with_root(root: &Path) -> Self {
        let mut devices = fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| is_accel_device(&entry.file_name().to_string_lossy()))
            .filter_map(|entry| npu_from_path(entry.path().join("device")))
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| left.pci_address.cmp(&right.pci_address));
        Self {
            devices,
            last_refresh: None,
        }
    }

    pub fn refresh(&mut self) {
        self.refresh_at(Instant::now());
    }

    fn refresh_at(&mut self, now: Instant) {
        if self
            .last_refresh
            .and_then(|previous| now.checked_duration_since(previous))
            .is_some_and(|elapsed| elapsed < MIN_REFRESH_INTERVAL)
        {
            return;
        }
        self.last_refresh = Some(now);
        for device in &mut self.devices {
            device.refresh_at(now);
        }
    }

    pub fn snapshot(&self) -> NpuSnapshot {
        NpuSnapshot {
            devices: self.devices.iter().map(NpuDevice::snapshot).collect(),
        }
    }
}

fn npu_from_path(path: PathBuf) -> Option<NpuDevice> {
    let vendor_id = read_trimmed(&path.join("vendor"))?.to_ascii_lowercase();
    let device_id = read_trimmed(&path.join("device"))?.to_ascii_lowercase();
    let uevent = read_trimmed(&path.join("uevent"))?;
    if vendor_id != INTEL_VENDOR_ID || uevent_value(&uevent, "DRIVER") != Some("intel_vpu") {
        return None;
    }
    Some(NpuDevice {
        path,
        pci_address: uevent_value(&uevent, "PCI_SLOT_NAME")?.into(),
        vendor_id,
        device_id,
        previous_busy: None,
        utilization_percent: None,
        current_frequency_mhz: None,
        max_frequency_mhz: None,
        memory_used_bytes: None,
    })
}

fn utilization_percent(previous: (u64, Instant), current: (u64, Instant)) -> Option<f32> {
    let busy_us = current.0.checked_sub(previous.0)?;
    let elapsed_us = current.1.checked_duration_since(previous.1)?.as_secs_f64() * 1_000_000.0;
    if elapsed_us <= 0.0 {
        return None;
    }
    Some((busy_us as f64 / elapsed_us * 100.0).clamp(0.0, 100.0) as f32)
}

fn is_accel_device(name: &str) -> bool {
    name.strip_prefix("accel").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
    })
}

fn uevent_value<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    raw.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().into())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_root() -> PathBuf {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("perfo-npu-{}-{id}", std::process::id()));
        let device = root.join("accel0/device");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&device).unwrap();
        fs::write(device.join("vendor"), "0x8086\n").unwrap();
        fs::write(device.join("device"), "0x7d1d\n").unwrap();
        fs::write(
            device.join("uevent"),
            "DRIVER=intel_vpu\nPCI_SLOT_NAME=0000:00:0b.0\n",
        )
        .unwrap();
        fs::write(device.join("npu_busy_time_us"), "1000000\n").unwrap();
        fs::write(device.join("npu_current_frequency_mhz"), "400\n").unwrap();
        fs::write(device.join("npu_max_frequency_mhz"), "1600\n").unwrap();
        fs::write(device.join("npu_memory_utilization"), "68722688\n").unwrap();
        root
    }

    #[test]
    fn monitor_discovers_intel_npu_and_calculates_usage() {
        let root = fixture_root();
        let start = Instant::now();
        let mut monitor = NpuMonitor::with_root(&root);

        monitor.refresh_at(start);
        assert_eq!(monitor.snapshot().devices[0].utilization_percent, None);

        fs::write(root.join("accel0/device/npu_busy_time_us"), "1250000\n").unwrap();
        monitor.refresh_at(start + Duration::from_millis(500));
        assert_eq!(monitor.snapshot().devices[0].utilization_percent, None);

        fs::write(root.join("accel0/device/npu_busy_time_us"), "1500000\n").unwrap();
        monitor.refresh_at(start + Duration::from_secs(1));

        let device = &monitor.snapshot().devices[0];
        assert_eq!(device.pci_address, "0000:00:0b.0");
        assert_eq!(device.vendor_id, "0x8086");
        assert_eq!(device.device_id, "0x7d1d");
        assert_eq!(device.utilization_percent, Some(50.0));
        assert_eq!(device.current_frequency_mhz, Some(400));
        assert_eq!(device.max_frequency_mhz, Some(1600));
        assert_eq!(device.memory_used_bytes, Some(68_722_688));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn monitor_ignores_non_intel_and_malformed_accelerators() {
        let root = fixture_root();
        fs::write(root.join("accel0/device/vendor"), "0x1002\n").unwrap();
        fs::rename(root.join("accel0"), root.join("accel-invalid")).unwrap();

        let monitor = NpuMonitor::with_root(&root);

        assert!(monitor.snapshot().devices.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn counter_reset_is_unknown_and_large_deltas_are_clamped() {
        let root = fixture_root();
        let start = Instant::now();
        let mut monitor = NpuMonitor::with_root(&root);
        monitor.refresh_at(start);

        fs::write(root.join("accel0/device/npu_busy_time_us"), "500000\n").unwrap();
        monitor.refresh_at(start + Duration::from_secs(1));
        assert_eq!(monitor.snapshot().devices[0].utilization_percent, None);

        fs::write(root.join("accel0/device/npu_busy_time_us"), "2500000\n").unwrap();
        monitor.refresh_at(start + Duration::from_secs(2));
        assert_eq!(
            monitor.snapshot().devices[0].utilization_percent,
            Some(100.0)
        );

        fs::remove_dir_all(root).unwrap();
    }
}

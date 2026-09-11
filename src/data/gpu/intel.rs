//! Intel GPUs, whose utilisation only exists as per-client DRM fdinfo
//! counters that have to be summed and differenced by hand.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use super::sysfs::is_drm_card;
use super::sysfs::read_trimmed;
use super::{GpuInfo, GpuProcessInfo, DRM_ENGINE_COUNT, INTEL_VENDOR_ID};

pub(super) struct IntelDrm {
    pdev: String,
    previous: Option<HashMap<u32, IntelEngineTimes>>,
    sampled_at: Option<Instant>,
    usage_percent: Option<f32>,
    processes: Vec<GpuProcessInfo>,
}

impl IntelDrm {
    pub(super) fn discover() -> Vec<Self> {
        let Ok(entries) = fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| is_drm_card(&entry.file_name().to_string_lossy()))
            .filter_map(intel_device_from_entry)
            .collect()
    }

    pub(super) fn refresh(&mut self, processes: bool) {
        let now = Instant::now();
        let current = read_drm_engine_times(&self.pdev);
        let elapsed = self
            .sampled_at
            .map(|sampled_at| now.duration_since(sampled_at).as_secs_f32())
            .unwrap_or(0.0);
        self.usage_percent = self.previous.as_ref().and_then(|previous| {
            engine_usage_percent(
                total_engine_times(previous),
                total_engine_times(&current),
                elapsed,
            )
        });
        self.processes = if processes {
            self.previous
                .as_ref()
                .map(|previous| process_gpu_usage(previous, &current, elapsed))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.previous = Some(current);
        self.sampled_at = Some(now);
    }

    pub(super) fn snapshot(&self) -> GpuInfo {
        GpuInfo {
            name: "Intel GPU".into(),
            vendor: "Intel".into(),
            usage_percent: self.usage_percent,
            memory_used_bytes: None,
            memory_total_bytes: None,
            temperature_c: None,
            power_w: None,
            processes: self.processes.clone(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct IntelEngineTimes {
    pub(super) values: [u64; DRM_ENGINE_COUNT],
}

impl IntelEngineTimes {
    fn add_assign(&mut self, other: Self) {
        for (total, value) in self.values.iter_mut().zip(other.values) {
            *total = total.saturating_add(value);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct IntelClientSample {
    pub(super) client_id: u64,
    pub(super) engines: IntelEngineTimes,
}

pub(super) fn intel_device_from_entry(entry: fs::DirEntry) -> Option<IntelDrm> {
    let device = entry.path().join("device");
    let raw = read_trimmed(&device.join("uevent"))?;
    if uevent_value(&raw, "DRIVER") != Some("i915") {
        return None;
    }
    let pdev = uevent_value(&raw, "PCI_SLOT_NAME")?.to_owned();
    if read_trimmed(&device.join("vendor")).as_deref() != Some(INTEL_VENDOR_ID) {
        return None;
    }
    Some(IntelDrm {
        pdev,
        previous: None,
        sampled_at: None,
        usage_percent: None,
        processes: Vec::new(),
    })
}

fn uevent_value<'a>(raw: &'a str, key: &str) -> Option<&'a str> {
    raw.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

pub(super) fn read_drm_engine_times(target_pdev: &str) -> HashMap<u32, IntelEngineTimes> {
    let mut totals = HashMap::new();
    let mut clients = HashSet::new();
    let Ok(processes) = fs::read_dir("/proc") else {
        return totals;
    };
    for process in processes.flatten() {
        accumulate_process_engine_times(&process.path(), target_pdev, &mut clients, &mut totals);
    }
    totals
}

fn accumulate_process_engine_times(
    process_path: &Path,
    target_pdev: &str,
    clients: &mut HashSet<(u32, u64)>,
    totals: &mut HashMap<u32, IntelEngineTimes>,
) {
    let Some(pid) = process_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.parse::<u32>().ok())
    else {
        return;
    };
    let Ok(file_descriptors) = fs::read_dir(process_path.join("fdinfo")) else {
        return;
    };
    for file_descriptor in file_descriptors.flatten() {
        accumulate_fd_engine_times(&file_descriptor.path(), pid, target_pdev, clients, totals);
    }
}

fn accumulate_fd_engine_times(
    fdinfo_path: &Path,
    pid: u32,
    target_pdev: &str,
    clients: &mut HashSet<(u32, u64)>,
    totals: &mut HashMap<u32, IntelEngineTimes>,
) {
    let Ok(raw) = fs::read_to_string(fdinfo_path) else {
        return;
    };
    let Some(sample) = parse_drm_fdinfo(&raw, target_pdev) else {
        return;
    };
    if clients.insert((pid, sample.client_id)) {
        totals.entry(pid).or_default().add_assign(sample.engines);
    }
}

pub(super) fn parse_drm_fdinfo(raw: &str, target_pdev: &str) -> Option<IntelClientSample> {
    let mut pdev = None;
    let mut client_id = None;
    let mut engines = IntelEngineTimes::default();
    for line in raw.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "drm-pdev" => pdev = Some(value),
            "drm-client-id" => client_id = value.parse().ok(),
            _ => {
                if let Some((index, time_ns)) = drm_engine_value(key, value) {
                    engines.values[index] = time_ns;
                }
            }
        }
    }
    (pdev == Some(target_pdev)).then_some(IntelClientSample {
        client_id: client_id?,
        engines,
    })
}

fn drm_engine_value(key: &str, value: &str) -> Option<(usize, u64)> {
    let index = match key {
        "drm-engine-render" => 0,
        "drm-engine-copy" => 1,
        "drm-engine-video" => 2,
        "drm-engine-video-enhance" => 3,
        "drm-engine-compute" => 4,
        _ => return None,
    };
    Some((index, value.strip_suffix(" ns")?.parse().ok()?))
}

pub(super) fn engine_usage_percent(
    previous: IntelEngineTimes,
    current: IntelEngineTimes,
    elapsed_seconds: f32,
) -> Option<f32> {
    if elapsed_seconds <= 0.0 {
        return None;
    }
    let max_delta = previous
        .values
        .into_iter()
        .zip(current.values)
        .map(|(old, new)| new.saturating_sub(old))
        .max()
        .unwrap_or(0);
    Some(
        (max_delta as f64 / (elapsed_seconds as f64 * 1_000_000_000.0) * 100.0).clamp(0.0, 100.0)
            as f32,
    )
}

fn total_engine_times(samples: &HashMap<u32, IntelEngineTimes>) -> IntelEngineTimes {
    samples
        .values()
        .fold(IntelEngineTimes::default(), |mut total, sample| {
            total.add_assign(*sample);
            total
        })
}

pub(super) fn process_gpu_usage(
    previous: &HashMap<u32, IntelEngineTimes>,
    current: &HashMap<u32, IntelEngineTimes>,
    elapsed_seconds: f32,
) -> Vec<GpuProcessInfo> {
    let mut processes: Vec<GpuProcessInfo> = current
        .iter()
        .filter_map(|(pid, current)| {
            let previous = previous.get(pid)?;
            let gpu_percent = engine_usage_percent(*previous, *current, elapsed_seconds)?;
            (gpu_percent > 0.0).then_some(GpuProcessInfo {
                pid: *pid,
                gpu_percent: Some(gpu_percent),
                memory_used_bytes: None,
            })
        })
        .collect();
    processes.sort_by(|a, b| {
        b.gpu_percent
            .unwrap_or_default()
            .total_cmp(&a.gpu_percent.unwrap_or_default())
    });
    processes
}

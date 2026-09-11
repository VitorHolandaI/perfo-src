use serde::Serialize;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use super::gpu::GpuMonitor;
use super::npu::NpuMonitor;

const GPU_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Serialize)]
pub struct WidgetGpuDevice {
    pub usage_percent: Option<f32>,
}

#[derive(Serialize)]
pub struct WidgetGpuSummary {
    pub devices: Vec<WidgetGpuDevice>,
}

#[derive(Serialize)]
pub struct WidgetNpuDevice {
    pub utilization_percent: Option<f32>,
}

#[derive(Serialize)]
pub struct WidgetNpuSummary {
    pub devices: Vec<WidgetNpuDevice>,
}

#[derive(Serialize)]
pub struct WidgetSummarySnapshot {
    pub overall_percent: f32,
    pub total_mem_bytes: u64,
    pub used_mem_bytes: u64,
    pub gpu: WidgetGpuSummary,
    pub npu: WidgetNpuSummary,
}

pub struct WidgetSummaryMonitor {
    sys: System,
    gpu: GpuMonitor,
    gpu_sampled_at: Option<Instant>,
    npu: NpuMonitor,
}

impl Default for WidgetSummaryMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetSummaryMonitor {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let mut gpu = GpuMonitor::new();
        gpu.refresh_summary();
        let mut monitor = Self {
            sys,
            gpu,
            gpu_sampled_at: Some(Instant::now()),
            npu: NpuMonitor::new(),
        };
        monitor.refresh();
        monitor
    }

    pub fn refresh(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let now = Instant::now();
        if gpu_sample_due(self.gpu_sampled_at, now) {
            self.gpu.refresh_summary();
            self.gpu_sampled_at = Some(now);
        }
        self.npu.refresh();
    }

    pub fn snapshot(&self) -> WidgetSummarySnapshot {
        WidgetSummarySnapshot {
            overall_percent: self.sys.global_cpu_usage(),
            total_mem_bytes: self.sys.total_memory(),
            used_mem_bytes: self.sys.used_memory(),
            gpu: WidgetGpuSummary {
                devices: self
                    .gpu
                    .snapshot()
                    .devices
                    .into_iter()
                    .map(|device| WidgetGpuDevice {
                        usage_percent: device.usage_percent,
                    })
                    .collect(),
            },
            npu: WidgetNpuSummary {
                devices: self
                    .npu
                    .snapshot()
                    .devices
                    .into_iter()
                    .map(|device| WidgetNpuDevice {
                        utilization_percent: device.utilization_percent,
                    })
                    .collect(),
            },
        }
    }
}

fn gpu_sample_due(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_none_or(|sampled_at| now.duration_since(sampled_at) >= GPU_SAMPLE_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn gpu_sampling_waits_for_summary_interval() {
        let started = Instant::now();
        assert!(gpu_sample_due(None, started));
        assert!(!gpu_sample_due(
            Some(started),
            started + Duration::from_secs(1)
        ));
        assert!(gpu_sample_due(Some(started), started + GPU_SAMPLE_INTERVAL));
    }
}

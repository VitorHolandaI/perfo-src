//! Picks a backend per card and keeps the previous sample for deltas.

use crate::data::nvidia::NvidiaBackend;

use super::amd::AmdDevice;
use super::intel::IntelDrm;
use super::GpuSnapshot;

enum Backend {
    Amd(AmdDevice),
    Intel(IntelDrm),
    Nvidia(NvidiaBackend),
}

pub struct GpuMonitor {
    backends: Vec<Backend>,
}

impl GpuMonitor {
    pub fn new() -> Self {
        let mut backends: Vec<Backend> = AmdDevice::discover()
            .into_iter()
            .map(Backend::Amd)
            .collect();
        backends.extend(IntelDrm::discover().into_iter().map(Backend::Intel));
        if let Some(nvidia) = NvidiaBackend::discover() {
            backends.push(Backend::Nvidia(nvidia));
        }
        Self { backends }
    }

    pub fn refresh(&mut self) {
        self.refresh_with_processes(true);
    }

    pub fn refresh_summary(&mut self) {
        self.refresh_with_processes(false);
    }

    fn refresh_with_processes(&mut self, processes: bool) {
        for backend in &mut self.backends {
            match backend {
                Backend::Intel(drm) => drm.refresh(processes),
                Backend::Nvidia(nvidia) => nvidia.refresh(processes),
                Backend::Amd(_) => {}
            }
        }
    }

    pub fn snapshot(&self) -> GpuSnapshot {
        GpuSnapshot {
            devices: self
                .backends
                .iter()
                .flat_map(|backend| match backend {
                    Backend::Amd(device) => vec![device.snapshot()],
                    Backend::Intel(drm) => vec![drm.snapshot()],
                    Backend::Nvidia(nvidia) => nvidia.snapshots(),
                })
                .collect(),
        }
    }
}

impl Default for GpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

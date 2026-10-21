//! Picks a backend per card and keeps the previous sample for deltas.

use crate::data::nvidia::NvidiaBackend;
use crate::data::nvidia_smi::SmiBackend;

use super::amd::AmdDevice;
use super::intel::IntelDrm;
use super::GpuSnapshot;

enum Backend {
    Amd(AmdDevice),
    Intel(IntelDrm),
    Nvidia(NvidiaBackend),
    /// NVIDIA without NVML, for the statically linked release build, where
    /// `dlopen` cannot work at all. See `data::nvidia_smi`.
    NvidiaSmi(SmiBackend),
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
        // NVML first: it reports per-process GPU utilisation, which the
        // nvidia-smi query interface has no column for. The release binary is
        // static musl, so its dlopen always fails and it lands on the child
        // process instead.
        if let Some(nvidia) = NvidiaBackend::discover() {
            backends.push(Backend::Nvidia(nvidia));
        } else if let Some(smi) = SmiBackend::discover() {
            backends.push(Backend::NvidiaSmi(smi));
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
                Backend::NvidiaSmi(smi) => smi.refresh(processes),
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
                    Backend::NvidiaSmi(smi) => smi.snapshots(),
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

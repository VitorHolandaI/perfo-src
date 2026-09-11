//! What each view needs collected, so one pass can serve a visible pane and
//! a background recording at the same time.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionProfile {
    Dashboard,
    Cpu,
    Io,
    Net,
    Mem,
    Disks,
    Gpu,
    History,
}

impl CollectionProfile {
    pub fn needs(self) -> CollectionNeeds {
        match self {
            Self::Dashboard => CollectionNeeds {
                cpu: true,
                cpu_details: true,
                cpu_temperatures: true,
                memory: true,
                disks: true,
                disk_details: true,
                io_wait: true,
                network: true,
                gpu: true,
                npu: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                ..CollectionNeeds::default()
            },
            Self::Cpu => CollectionNeeds {
                cpu: true,
                cpu_details: true,
                cpu_temperatures: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                process_tasks: true,
                process_affinity: true,
                ..CollectionNeeds::default()
            },
            Self::Io => CollectionNeeds {
                disks: true,
                disk_details: true,
                disk_temperatures: true,
                io_wait: true,
                processes: true,
                process_io: true,
                ..CollectionNeeds::default()
            },
            Self::Net => CollectionNeeds {
                processes: true,
                network: true,
                network_processes: true,
                network_listeners: true,
                ..CollectionNeeds::default()
            },
            Self::Mem => CollectionNeeds {
                memory: true,
                memory_details: true,
                processes: true,
                process_memory: true,
                ..CollectionNeeds::default()
            },
            Self::Disks => CollectionNeeds {
                disks: true,
                disk_temperatures: true,
                ..CollectionNeeds::default()
            },
            Self::Gpu => CollectionNeeds {
                memory: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                gpu: true,
                gpu_processes: true,
                npu: true,
                ..CollectionNeeds::default()
            },
            Self::History => CollectionNeeds {
                cpu: true,
                memory: true,
                processes: true,
                process_cpu: true,
                process_memory: true,
                process_tasks: true,
                process_affinity: true,
                process_io: true,
                disks: true,
                network: true,
                network_processes: true,
                gpu: true,
                gpu_processes: true,
                npu: true,
                ..CollectionNeeds::default()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollectionNeeds {
    pub cpu: bool,
    pub cpu_details: bool,
    pub cpu_temperatures: bool,
    pub memory: bool,
    pub memory_details: bool,
    pub processes: bool,
    pub process_cpu: bool,
    pub process_memory: bool,
    pub process_tasks: bool,
    pub process_affinity: bool,
    pub process_io: bool,
    pub disks: bool,
    pub disk_details: bool,
    pub disk_temperatures: bool,
    pub io_wait: bool,
    pub network: bool,
    pub network_processes: bool,
    pub network_listeners: bool,
    pub gpu: bool,
    pub gpu_processes: bool,
    pub npu: bool,
    pub fans: bool,
}

impl CollectionNeeds {
    pub fn full() -> Self {
        Self {
            cpu: true,
            cpu_details: true,
            cpu_temperatures: true,
            memory: true,
            memory_details: true,
            processes: true,
            process_cpu: true,
            process_memory: true,
            process_tasks: true,
            process_affinity: true,
            process_io: true,
            disks: true,
            disk_temperatures: true,
            disk_details: true,
            io_wait: true,
            network: true,
            network_processes: true,
            network_listeners: true,
            gpu: true,
            gpu_processes: true,
            npu: true,
            fans: true,
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            cpu: self.cpu || other.cpu,
            cpu_details: self.cpu_details || other.cpu_details,
            cpu_temperatures: self.cpu_temperatures || other.cpu_temperatures,
            memory: self.memory || other.memory,
            memory_details: self.memory_details || other.memory_details,
            processes: self.processes || other.processes,
            process_cpu: self.process_cpu || other.process_cpu,
            process_memory: self.process_memory || other.process_memory,
            process_tasks: self.process_tasks || other.process_tasks,
            process_affinity: self.process_affinity || other.process_affinity,
            process_io: self.process_io || other.process_io,
            disks: self.disks || other.disks,
            disk_details: self.disk_details || other.disk_details,
            disk_temperatures: self.disk_temperatures || other.disk_temperatures,
            io_wait: self.io_wait || other.io_wait,
            network: self.network || other.network,
            network_processes: self.network_processes || other.network_processes,
            network_listeners: self.network_listeners || other.network_listeners,
            gpu: self.gpu || other.gpu,
            gpu_processes: self.gpu_processes || other.gpu_processes,
            npu: self.npu || other.npu,
            fans: self.fans || other.fans,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingMask {
    pub cpu: bool,
    pub mem: bool,
    pub io: bool,
    pub net: bool,
    pub gpu: bool,
    pub npu: bool,
}

impl RecordingMask {
    pub const ALL: Self = Self {
        cpu: true,
        mem: true,
        io: true,
        net: true,
        gpu: true,
        npu: true,
    };

    pub const fn is_empty(&self) -> bool {
        !self.cpu && !self.mem && !self.io && !self.net && !self.gpu && !self.npu
    }

    pub const fn count(&self) -> usize {
        let mut n = 0;
        if self.cpu {
            n += 1;
        }
        if self.mem {
            n += 1;
        }
        if self.io {
            n += 1;
        }
        if self.net {
            n += 1;
        }
        if self.gpu {
            n += 1;
        }
        if self.npu {
            n += 1;
        }
        n
    }

    pub fn summary(&self) -> String {
        if self.count() == 6 {
            return "ALL".to_string();
        }
        let mut parts = Vec::new();
        if self.cpu {
            parts.push("CPU");
        }
        if self.mem {
            parts.push("MEM");
        }
        if self.io {
            parts.push("IO");
        }
        if self.net {
            parts.push("NET");
        }
        if self.gpu {
            parts.push("GPU");
        }
        if self.npu {
            parts.push("NPU");
        }
        if parts.is_empty() {
            "NONE".to_string()
        } else {
            parts.join(",")
        }
    }

    fn to_needs(self) -> CollectionNeeds {
        CollectionNeeds {
            cpu: self.cpu,
            cpu_details: self.cpu,
            processes: self.cpu || self.mem || self.io || self.net || self.gpu,
            process_cpu: self.cpu,
            process_tasks: self.cpu,
            process_affinity: self.cpu,
            memory: self.mem,
            process_memory: self.mem,
            disks: self.io,
            process_io: self.io,
            network: self.net,
            network_processes: self.net,
            gpu: self.gpu,
            gpu_processes: self.gpu,
            npu: self.npu,
            ..CollectionNeeds::default()
        }
    }
}

impl Default for RecordingMask {
    fn default() -> Self {
        Self::ALL
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollectionPlan {
    pub visible: CollectionProfile,
    pub recording: bool,
    pub recording_mask: RecordingMask,
}

impl CollectionPlan {
    pub const fn new(visible: CollectionProfile, recording: bool) -> Self {
        Self {
            visible,
            recording,
            recording_mask: RecordingMask::ALL,
        }
    }

    pub const fn with_recording_mask(
        visible: CollectionProfile,
        recording: bool,
        recording_mask: RecordingMask,
    ) -> Self {
        Self {
            visible,
            recording,
            recording_mask,
        }
    }

    pub const fn for_profile(visible: CollectionProfile) -> Self {
        Self {
            visible,
            recording: false,
            recording_mask: RecordingMask::ALL,
        }
    }

    pub fn needs(self) -> CollectionNeeds {
        if self.recording {
            let base_needs = match self.visible {
                CollectionProfile::History => CollectionNeeds::default(),
                other => other.needs(),
            };
            base_needs.union(self.recording_mask.to_needs())
        } else {
            self.visible.needs()
        }
    }
}

impl Default for CollectionPlan {
    fn default() -> Self {
        Self::for_profile(CollectionProfile::Dashboard)
    }
}

impl From<CollectionProfile> for CollectionPlan {
    fn from(profile: CollectionProfile) -> Self {
        Self::for_profile(profile)
    }
}

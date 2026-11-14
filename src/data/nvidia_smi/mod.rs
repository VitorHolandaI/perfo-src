//! NVIDIA metrics without NVML, for the statically linked build.
//!
//! The published binary is linked against musl with `crt-static`, and a
//! static musl program has no dynamic loader: its `dlopen` is a stub that
//! always returns NULL. Verified on 2026-10-21 by building the same probe
//! both ways, where the musl build failed to open even `libc.so.6`. So
//! `nvidia.rs` cannot reach `libnvidia-ml.so.1` in a release build, and this
//! asks the `nvidia-smi` that ships with every driver instead.
//!
//! The sample runs as a child process that is never waited on: a wedged
//! driver is exactly when someone opens a monitor, and blocking the refresh
//! loop on it would freeze the whole UI. One sample is in flight at a time
//! and its result is collected on a later refresh.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::gpu::{GpuInfo, GpuProcessInfo};
use crate::bounded_child::wait_bounded;

/// How often a new sample is started. The driver powers the card down
/// between queries, which `nvidia-smi -l` would prevent, so this spawns a
/// one-shot each time rather than holding a session open.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// How long one sample may take before it is abandoned. The pending slot is
/// single, so a query that never answers would hold it and stop every later
/// sample: the series would freeze at its last values for the life of the
/// process. Generous against a slow-but-working driver, and still bounded.
const SAMPLE_DEADLINE: Duration = Duration::from_secs(5);

/// How long the one startup query may take. `GpuMonitor::new` runs before the
/// collector's first snapshot, so a wedged driver here leaves the widget blank
/// rather than merely GPU-less. Giving up costs the card for this process;
/// blocking costs every metric.
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(3);

const GPU_QUERY: &str = "name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw";
const APP_QUERY: &str = "pid,used_gpu_memory";
const CSV_FORMAT: &str = "csv,noheader,nounits";

const NVIDIA_PROC_DIR: &str = "/proc/driver/nvidia";

mod rows;

pub(crate) use rows::{parse_app_rows, parse_gpu_rows, SmiDevice};

/// A sample that was started and not yet collected.
struct PendingSample {
    gpus: Child,
    apps: Option<Child>,
}

pub(crate) struct SmiBackend {
    devices: Vec<SmiDevice>,
    processes: Vec<GpuProcessInfo>,
    pending: Option<PendingSample>,
    last_started: Instant,
}

/// Whether the proprietary NVIDIA kernel driver is loaded.
///
/// Checked before anything is spawned, so a machine with no NVIDIA card
/// pays nothing. `/proc/driver/nvidia` is created by that driver and is
/// what both NVML and nvidia-smi need in order to answer at all; a host
/// with nvidia-utils installed but no card, like an Intel laptop that
/// pulled the package in as a dependency, has the binary and not this.
fn nvidia_driver_loaded() -> bool {
    std::path::Path::new(NVIDIA_PROC_DIR).is_dir()
}

impl SmiBackend {
    /// Present only when nvidia-smi answers with at least one card.
    ///
    /// Bounded by `DISCOVERY_DEADLINE`. It happens once, at startup, and only
    /// on a host with the driver loaded, which is exactly the host where
    /// `nvidia-smi` can wedge; an unbounded wait here held `GpuMonitor::new`
    /// and the collector never produced a first snapshot.
    pub(crate) fn discover() -> Option<Self> {
        if !nvidia_driver_loaded() {
            return None;
        }
        let mut child = spawn_query(&("--query-gpu=".to_string() + GPU_QUERY))?;
        if !wait_bounded(&mut child, DISCOVERY_DEADLINE).is_some_and(|s| s.success()) {
            return None;
        }
        let devices = parse_gpu_rows(&read_stdout(&mut child));
        (!devices.is_empty()).then(|| Self {
            devices,
            processes: Vec::new(),
            pending: None,
            last_started: Instant::now(),
        })
    }

    /// Collects a finished sample and starts the next one when due.
    ///
    /// Never waits: a call that finds the previous sample still running
    /// leaves the last known values in place and returns.
    pub(crate) fn refresh(&mut self, processes: bool) {
        self.collect_finished();
        if self.pending.is_none() && self.last_started.elapsed() >= SAMPLE_INTERVAL {
            self.start_sample(processes);
        }
    }

    fn start_sample(&mut self, processes: bool) {
        let Some(gpus) = spawn_query(&("--query-gpu=".to_string() + GPU_QUERY)) else {
            return;
        };
        let apps = processes
            .then(|| spawn_query(&("--query-compute-apps=".to_string() + APP_QUERY)))
            .flatten();
        self.pending = Some(PendingSample { gpus, apps });
        self.last_started = Instant::now();
    }

    fn collect_finished(&mut self) {
        if self.pending.is_none() {
            return;
        }
        // Out of budget: drop the sample rather than the series. The pipes are
        // only read once both children exit, so a child that filled its pipe
        // would block on the write, never exit, and hold the slot forever.
        if self.last_started.elapsed() >= SAMPLE_DEADLINE {
            if let Some(mut pending) = self.pending.take() {
                abandon(&mut pending);
            }
            return;
        }
        let Some(pending) = &mut self.pending else {
            return;
        };
        if !matches!(pending.gpus.try_wait(), Ok(Some(_))) {
            return;
        }
        if let Some(apps) = &mut pending.apps {
            if !matches!(apps.try_wait(), Ok(Some(_))) {
                return;
            }
        }
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        let devices = parse_gpu_rows(&read_stdout(&mut pending.gpus));
        if !devices.is_empty() {
            self.devices = devices;
        }
        self.processes = match &mut pending.apps {
            Some(apps) => parse_app_rows(&read_stdout(apps)),
            None => Vec::new(),
        };
    }

    pub(crate) fn snapshots(&self) -> Vec<GpuInfo> {
        self.devices
            .iter()
            .enumerate()
            .map(|(index, device)| GpuInfo {
                name: device.name.clone(),
                vendor: "NVIDIA".into(),
                usage_percent: device.usage_percent,
                memory_used_bytes: device.memory_used_bytes,
                memory_total_bytes: device.memory_total_bytes,
                temperature_c: device.temperature_c,
                power_w: device.power_w,
                // nvidia-smi does not say which card a process is on in this
                // query, so they are charged to the first one rather than
                // repeated under every card.
                processes: if index == 0 {
                    self.processes.clone()
                } else {
                    Vec::new()
                },
            })
            .collect()
    }
}

/// Reaps the child on drop so a shutdown mid-sample leaves no zombie.
impl Drop for SmiBackend {
    fn drop(&mut self) {
        if let Some(pending) = &mut self.pending {
            abandon(pending);
        }
    }
}

/// Kills a sample's children and reaps them, so an abandoned query leaves no
/// zombie and frees the single pending slot.
fn abandon(pending: &mut PendingSample) {
    let _ = pending.gpus.kill();
    let _ = pending.gpus.wait();
    if let Some(apps) = &mut pending.apps {
        let _ = apps.kill();
        let _ = apps.wait();
    }
}

fn spawn_query(query: &str) -> Option<Child> {
    Command::new("nvidia-smi")
        .args([query, &format!("--format={CSV_FORMAT}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Only called once a child has exited, so its pipe holds the whole answer.
///
/// The queries answer in a few hundred bytes, far under the pipe buffer, but
/// that is a measurement and not a guarantee: a child whose output filled the
/// pipe would block on the write and never exit, which is why
/// `SAMPLE_DEADLINE` abandons it rather than leaving the slot held.
fn read_stdout(child: &mut Child) -> String {
    let mut buffer = String::new();
    if let Some(stdout) = &mut child.stdout {
        let _ = stdout.read_to_string(&mut buffer);
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pending slot is single, so a stalled query that keeps it stops every
    /// later sample and freezes the series for the life of the process.
    #[test]
    fn a_stalled_sample_frees_the_slot_instead_of_freezing_the_series() {
        let gpus = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep is not available");
        let mut backend = SmiBackend {
            devices: vec![SmiDevice {
                name: "Quadro M2000M".to_string(),
                ..SmiDevice::default()
            }],
            processes: Vec::new(),
            pending: Some(PendingSample { gpus, apps: None }),
            last_started: Instant::now() - SAMPLE_DEADLINE - Duration::from_secs(1),
        };

        backend.collect_finished();

        assert!(
            backend.pending.is_none(),
            "the stalled sample kept the slot"
        );
        // The last known values stay: an abandoned sample is missing data, not
        // a reason to blank the card.
        assert_eq!(backend.devices.len(), 1);
        assert_eq!(backend.devices[0].name, "Quadro M2000M");
    }

    /// A sample still inside its budget is left alone, so an ordinary slow
    /// answer is not thrown away.
    #[test]
    fn a_sample_inside_its_budget_is_left_running() {
        let gpus = Command::new("sleep")
            .arg("60")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep is not available");
        let mut backend = SmiBackend {
            devices: Vec::new(),
            processes: Vec::new(),
            pending: Some(PendingSample { gpus, apps: None }),
            last_started: Instant::now(),
        };

        backend.collect_finished();

        assert!(
            backend.pending.is_some(),
            "a sample in budget was abandoned"
        );
    }

    #[test]
    fn a_host_without_the_driver_spawns_nothing() {
        // This machine has nvidia-smi from a package dependency and Intel
        // graphics. Discovery must not pay for a process on it.
        assert_eq!(
            nvidia_driver_loaded(),
            std::path::Path::new("/proc/driver/nvidia").is_dir()
        );
        if !nvidia_driver_loaded() {
            assert!(SmiBackend::discover().is_none());
        }
    }

    /// A backend holding a parsed sample, without spawning anything.
    fn backend_with(devices: Vec<SmiDevice>, processes: Vec<GpuProcessInfo>) -> SmiBackend {
        SmiBackend {
            devices,
            processes,
            pending: None,
            last_started: Instant::now(),
        }
    }

    #[test]
    fn snapshots_carry_every_card_and_charge_processes_once() {
        let backend = backend_with(
            parse_gpu_rows(
                "Quadro M2000M, 37, 512, 4096, 54, [N/A]\n\
                 Tesla K80, 5, 0, 11441, 31, 26.5\n",
            ),
            parse_app_rows("3140, 1024\n"),
        );
        let snapshots = backend.snapshots();

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].name, "Quadro M2000M");
        assert_eq!(snapshots[0].vendor, "NVIDIA");
        assert_eq!(snapshots[0].usage_percent, Some(37.0));
        assert_eq!(snapshots[0].power_w, None);
        assert_eq!(snapshots[0].processes.len(), 1);
        assert!(
            snapshots[1].processes.is_empty(),
            "the query does not say which card a process is on, so it is not repeated"
        );
    }

    #[test]
    fn a_refresh_that_finds_no_sample_keeps_the_last_values() {
        // The point of not waiting: a slow or wedged nvidia-smi leaves the
        // previous reading on screen instead of blanking the page.
        let mut backend = backend_with(
            parse_gpu_rows("Quadro M2000M, 37, 512, 4096, 54, 22.1\n"),
            Vec::new(),
        );
        backend.collect_finished();
        assert_eq!(backend.devices[0].usage_percent, Some(37.0));
    }
}

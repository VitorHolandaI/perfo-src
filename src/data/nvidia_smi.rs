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
use crate::units::clamp_percent;

/// How often a new sample is started. The driver powers the card down
/// between queries, which `nvidia-smi -l` would prevent, so this spawns a
/// one-shot each time rather than holding a session open.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

const GPU_QUERY: &str = "name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw";
const APP_QUERY: &str = "pid,used_gpu_memory";
const CSV_FORMAT: &str = "csv,noheader,nounits";

const NVIDIA_PROC_DIR: &str = "/proc/driver/nvidia";

/// One card as `nvidia-smi` reports it.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct SmiDevice {
    pub(crate) name: String,
    pub(crate) usage_percent: Option<f32>,
    pub(crate) memory_used_bytes: Option<u64>,
    pub(crate) memory_total_bytes: Option<u64>,
    pub(crate) temperature_c: Option<f32>,
    pub(crate) power_w: Option<f32>,
}

/// A field `nvidia-smi` could not answer.
///
/// It prints these in brackets rather than leaving the column empty, and the
/// exact wording varies by driver and by card: the Quadro M2000M has no
/// power sensor and answers `[N/A]` for `power.draw`.
fn optional_field(raw: &str) -> Option<&str> {
    let value = raw.trim();
    if value.is_empty() || value.starts_with('[') {
        return None;
    }
    Some(value)
}

fn percent_field(raw: &str) -> Option<f32> {
    optional_field(raw)?.parse().ok().map(clamp_percent)
}

/// Memory columns come out in MiB because the query asks for `nounits`.
fn mib_field(raw: &str) -> Option<u64> {
    optional_field(raw)?
        .parse::<u64>()
        .ok()
        .map(|mib| mib * 1024 * 1024)
}

fn float_field(raw: &str) -> Option<f32> {
    optional_field(raw)?.parse().ok()
}

/// One `SmiDevice` per line of `--query-gpu`, in the order the GPU_QUERY
/// columns are listed.
pub(crate) fn parse_gpu_rows(stdout: &str) -> Vec<SmiDevice> {
    stdout
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split(',').collect();
            // A name is the one column without which the row says nothing.
            let name = optional_field(columns.first()?)?;
            Some(SmiDevice {
                name: name.to_string(),
                usage_percent: columns.get(1).and_then(|c| percent_field(c)),
                memory_used_bytes: columns.get(2).and_then(|c| mib_field(c)),
                memory_total_bytes: columns.get(3).and_then(|c| mib_field(c)),
                temperature_c: columns.get(4).and_then(|c| float_field(c)),
                power_w: columns.get(5).and_then(|c| float_field(c)),
            })
        })
        .collect()
}

/// Processes holding GPU memory, from `--query-compute-apps`.
///
/// `gpu_percent` stays empty: NVML reports per-process utilisation and
/// nvidia-smi's query interface has no column for it. Memory is the part
/// that survives the fallback.
pub(crate) fn parse_app_rows(stdout: &str) -> Vec<GpuProcessInfo> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut columns = line.split(',');
            let pid = optional_field(columns.next()?)?.parse().ok()?;
            Some(GpuProcessInfo {
                pid,
                gpu_percent: None,
                memory_used_bytes: columns.next().and_then(mib_field),
            })
        })
        .collect()
}

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
    /// This one query is allowed to block: it happens once, at startup,
    /// before anything is on screen, and only on a host that has the
    /// driver loaded.
    pub(crate) fn discover() -> Option<Self> {
        if !nvidia_driver_loaded() {
            return None;
        }
        let output = Command::new("nvidia-smi")
            .args([
                "--query-gpu=".to_string() + GPU_QUERY,
                format!("--format={CSV_FORMAT}"),
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let devices = parse_gpu_rows(&String::from_utf8_lossy(&output.stdout));
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
            let _ = pending.gpus.kill();
            let _ = pending.gpus.wait();
            if let Some(apps) = &mut pending.apps {
                let _ = apps.kill();
                let _ = apps.wait();
            }
        }
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

/// The queries answer in a few hundred bytes, well under the pipe buffer,
/// so the child has already exited and nothing can deadlock here.
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

    #[test]
    fn a_working_card_fills_every_column() {
        let rows = parse_gpu_rows("Quadro M2000M, 37, 512, 4096, 54, 22.11\n");
        assert_eq!(
            rows,
            vec![SmiDevice {
                name: "Quadro M2000M".to_string(),
                usage_percent: Some(37.0),
                memory_used_bytes: Some(512 * 1024 * 1024),
                memory_total_bytes: Some(4096 * 1024 * 1024),
                temperature_c: Some(54.0),
                power_w: Some(22.11),
            }]
        );
    }

    #[test]
    fn a_card_without_a_power_sensor_still_reports_the_rest() {
        // The Quadro M2000M answers [N/A] for power.draw, and a row dropped
        // over one missing column would lose the whole card.
        let rows = parse_gpu_rows("Quadro M2000M, 0, 1, 4096, 48, [N/A]\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].power_w, None);
        assert_eq!(rows[0].temperature_c, Some(48.0));
    }

    #[test]
    fn not_supported_reads_the_same_as_not_available() {
        let rows = parse_gpu_rows("Tesla K80, [Not Supported], 0, 11441, 31, 26.5\n");
        assert_eq!(rows[0].usage_percent, None);
        assert_eq!(rows[0].memory_total_bytes, Some(11441 * 1024 * 1024));
    }

    #[test]
    fn every_card_gets_its_own_row() {
        let rows = parse_gpu_rows(
            "NVIDIA RTX A4000, 10, 100, 16376, 40, 30.0\n\
             NVIDIA RTX A4000, 90, 200, 16376, 70, 120.0\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].usage_percent, Some(90.0));
    }

    #[test]
    fn a_driver_error_message_is_not_a_card() {
        // What this machine prints, having nvidia-smi but no driver loaded.
        let rows = parse_gpu_rows(
            "NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.\n",
        );
        assert_eq!(rows.len(), 1, "one comma-less line parses as one row");
        assert_eq!(rows[0].usage_percent, None);
        assert_eq!(rows[0].memory_total_bytes, None);
    }

    #[test]
    fn utilisation_above_a_hundred_is_clamped() {
        let rows = parse_gpu_rows("GPU, 140, 0, 1, 0, 0\n");
        assert_eq!(rows[0].usage_percent, Some(100.0));
    }

    #[test]
    fn compute_apps_carry_memory_but_not_utilisation() {
        let apps = parse_app_rows("3140, 1024\n9982, 256\n");
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].pid, 3140);
        assert_eq!(apps[0].memory_used_bytes, Some(1024 * 1024 * 1024));
        assert_eq!(
            apps[0].gpu_percent, None,
            "nvidia-smi has no per-process utilisation column"
        );
    }

    #[test]
    fn an_app_row_without_a_pid_is_dropped() {
        assert!(parse_app_rows("\n[N/A], 10\n").is_empty());
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

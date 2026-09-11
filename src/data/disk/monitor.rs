//! Per-device rate tracking and mount discovery across refreshes.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::time::Instant;

use sysinfo::Disks;

use crate::data::psi;

use super::parse::{d_sectors, diskstats_from, io_stats_from, RawCounters, BYTES_PER_SECTOR};
use super::rate;
use super::temp::nvme_temp_c;
use super::{DiskInfo, DiskIoStats, HISTORY_SAMPLES};

pub struct DiskMonitor {
    disks: Disks,
    /// When the last refresh happened; drives the delta->rate conversion.
    last_refresh: Option<Instant>,
    prev_stats: HashMap<String, RawCounters>,
    io_stats: HashMap<String, DiskIoStats>,
    usage_rates: HashMap<String, (u64, u64)>,
    /// PSI "some" I/O pressure (10s/60s/300s) from /proc/pressure/io.
    io_pressure: [f64; 3],
    /// friendly dm name -> dm-N (e.g. "root" -> "dm-0"); dm-N is what
    /// exists under /sys/block for temperature lookups.
    dm_aliases: HashMap<String, String>,
    /// Per-device read/write byte-rate history (diskstats-derived), newest
    /// last; feeds the sparklines in the IO pane.
    history: HashMap<String, (VecDeque<f32>, VecDeque<f32>)>,
}

impl DiskMonitor {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            last_refresh: None,
            prev_stats: HashMap::new(),
            io_stats: HashMap::new(),
            usage_rates: HashMap::new(),
            io_pressure: [0.0; crate::units::PSI_WINDOWS],
            dm_aliases: HashMap::new(),
            history: HashMap::new(),
        }
    }

    pub fn refresh(&mut self) {
        self.refresh_with_details(true);
    }

    pub fn refresh_summary(&mut self) {
        self.refresh_with_details(false);
    }

    fn refresh_with_details(&mut self, details: bool) {
        // refresh(false) refreshes everything, including the io_usage deltas
        // that Disk::usage() reports as "since the last refresh".
        self.disks.refresh(false);
        let now = Instant::now();
        let elapsed = self
            .last_refresh
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let raw = std::fs::read_to_string("/proc/diskstats").unwrap_or_default();
        let cur = diskstats_from(&raw);
        self.usage_rates = self
            .disks
            .list()
            .iter()
            .map(|disk| {
                let name = disk.name().to_string_lossy();
                let key = name.rsplit('/').next().unwrap_or(&name).to_string();
                let usage = disk.usage();
                (
                    key,
                    (
                        rate(usage.read_bytes, elapsed),
                        rate(usage.written_bytes, elapsed),
                    ),
                )
            })
            .collect();
        // device-mapper devices show as dm-N; resolve to the friendly name
        // (e.g. dm-0 -> "root") so they match sysinfo's /dev/mapper/root.
        let mut dm_aliases = HashMap::new();
        let cur: HashMap<String, RawCounters> = cur
            .into_iter()
            .map(|(dev, counters)| {
                let key = if let Some(n) = dev.strip_prefix("dm-") {
                    let friendly = std::fs::read_to_string(format!("/sys/block/dm-{n}/dm/name"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or(dev.clone());
                    dm_aliases.insert(friendly.clone(), dev.clone());
                    friendly
                } else {
                    dev.clone()
                };
                (key, counters)
            })
            .collect();
        self.dm_aliases = dm_aliases;
        self.io_stats = if details {
            cur.iter()
                .filter_map(|(name, c)| {
                    self.prev_stats
                        .get(name)
                        .map(|p| (name.clone(), io_stats_from(p, c, elapsed)))
                })
                .collect()
        } else {
            HashMap::new()
        };
        // Byte-rate history for the sparklines, from the same diskstats
        // deltas as the table columns (not sysinfo, which lags a refresh).
        if details {
            for (name, c) in cur.iter() {
                if let Some(p) = self.prev_stats.get(name) {
                    let rb = d_sectors(p.sectors_read, c.sectors_read) * BYTES_PER_SECTOR;
                    let wb = d_sectors(p.sectors_written, c.sectors_written) * BYTES_PER_SECTOR;
                    let (rq, wq) = self
                        .history
                        .entry(name.clone())
                        .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                    push_capped(
                        rq,
                        rb as f32 / elapsed.max(crate::units::MIN_ELAPSED_SECS),
                        HISTORY_SAMPLES,
                    );
                    push_capped(
                        wq,
                        wb as f32 / elapsed.max(crate::units::MIN_ELAPSED_SECS),
                        HISTORY_SAMPLES,
                    );
                }
            }
            self.history.retain(|name, _| cur.contains_key(name));
            let (p10, p60, p300) = psi::some("io");
            self.io_pressure = [p10, p60, p300];
        } else {
            self.history.clear();
            self.io_pressure = [0.0; crate::units::PSI_WINDOWS];
        }
        self.prev_stats = cur;
        self.last_refresh = Some(now);
    }

    /// PSI I/O pressure "some" averages (10s/60s/300s).
    pub fn io_pressure(&self) -> [f64; 3] {
        self.io_pressure
    }

    /// Per-device read/write rate history (newest last) for sparklines.
    pub fn history(&self) -> &HashMap<String, (VecDeque<f32>, VecDeque<f32>)> {
        &self.history
    }

    pub fn snapshot(&self) -> Vec<DiskInfo> {
        self.snapshot_with_temperatures(true)
    }

    pub fn snapshot_with_temperatures(&self, temperatures: bool) -> Vec<DiskInfo> {
        const REAL_FS: [&str; 9] = [
            "btrfs", "vfat", "ext4", "xfs", "f2fs", "ntfs", "zfs", "exfat", "ext2",
        ];
        self.disks
            .list()
            .iter()
            .filter(|d| {
                let fs = d.file_system().to_string_lossy();
                REAL_FS.contains(&fs.as_ref())
            })
            .map(|d| {
                let u = d.usage();
                let total = d.total_space();
                let available = d.available_space();
                let used = total.saturating_sub(available);
                let name = d.name().to_string_lossy().into_owned();
                let key = name.rsplit('/').next().unwrap_or(&name).to_string();
                let io = self.io_stats.get(&key).cloned().unwrap_or_default();
                DiskInfo {
                    name,
                    mount: d.mount_point().to_string_lossy().into_owned(),
                    fs: d.file_system().to_string_lossy().into_owned(),
                    total_bytes: total,
                    available_bytes: available,
                    used_bytes: used,
                    percent: if total > 0 {
                        used as f32 / total as f32 * 100.0
                    } else {
                        0.0
                    },
                    read_bps: self.usage_rates.get(&key).map(|rates| rates.0).unwrap_or(0),
                    write_bps: self.usage_rates.get(&key).map(|rates| rates.1).unwrap_or(0),
                    total_read_bytes: u.total_read_bytes,
                    total_written_bytes: u.total_written_bytes,
                    temp_c: temperatures
                        .then(|| {
                            nvme_temp_c(
                                &Path::new("/sys/block")
                                    .join(self.dm_aliases.get(key.as_str()).unwrap_or(&key)),
                            )
                        })
                        .flatten(),
                    io,
                }
            })
            .collect()
    }
}

impl Default for DiskMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Push into a capped ring buffer.
pub(super) fn push_capped(q: &mut VecDeque<f32>, v: f32, cap: usize) {
    q.push_back(v);
    if q.len() > cap {
        q.pop_front();
    }
}

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use sysinfo::Disks;

use super::psi;

/// Bytes in one diskstats sector (kernel convention, independent of the
/// device's logical block size).
const BYTES_PER_SECTOR: u64 = 512;
/// KiB in bytes.
const BYTES_PER_KIB: u64 = 1024;
/// Milliseconds per second.
const MS_PER_SEC: f32 = 1000.0;
/// /proc/diskstats fields read into RawCounters (17 = discard + flush data).
const DISKSTAT_FIELDS: usize = 17;
/// Minimum fields a line must have (pre-4.18 kernels lack discard/flush).
const DISKSTAT_MIN_FIELDS: usize = 11;

/// Per-disk I/O rates derived from /proc/diskstats deltas (iostat-style).
#[derive(Clone, Serialize, Default)]
pub struct DiskIoStats {
    /// Read/write operations per second (IOPS, after merges).
    pub r_s: u64,
    pub w_s: u64,
    /// Average read/write latency in ms, queue time included. The number
    /// that actually tells you the disk is slow: NVMe healthy < 1ms,
    /// > 10ms = queueing hard.
    pub r_await_ms: f32,
    pub w_await_ms: f32,
    /// Average number of requests in flight (weighted time / interval).
    pub queue_avg: f32,
    /// % of the interval with at least one I/O in flight. On NVMe this is
    /// NOT saturation — a drive with 16 queues reports 100% with one
    /// request; trust await + queue instead.
    pub busy_pct: f32,
    /// % of requests that were merged with neighbours (sequential-ish).
    pub read_merge_pct: f32,
    pub write_merge_pct: f32,
    /// Average request size in KiB: 128+ = sequential, 4-16 = random.
    pub read_req_kib: f32,
    pub write_req_kib: f32,
    /// TRIM/discard operations per second (NVMe + fstrim.timer).
    pub d_s: u64,
    /// Average discard size in KiB (small discards = fragmented fstrim).
    pub discard_req_kib: f32,
    /// fsync/fdatasync completions per second (DB commit cadence).
    pub flush_s: u64,
}

#[derive(Clone, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub fs: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub percent: f32,
    /// Read rate since the last refresh (bytes/second).
    pub read_bps: u64,
    /// Write rate since the last refresh (bytes/second).
    pub write_bps: u64,
    /// Cumulative bytes read since boot (from /proc/diskstats).
    pub total_read_bytes: u64,
    pub total_written_bytes: u64,
    /// NVMe/SATA device temperature in °C (from hwmon), when available.
    pub temp_c: Option<f32>,
    #[serde(flatten)]
    pub io: DiskIoStats,
}

/// Bytes per second from a refresh-interval delta. Shared with net.rs.
pub(crate) fn rate(delta: u64, elapsed_secs: f32) -> u64 {
    if elapsed_secs <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed_secs as f64) as u64
    }
}

/// Raw per-device counters from /proc/diskstats (kernel iostats fields).
#[derive(Clone, Default)]
struct RawCounters {
    reads: u64,
    reads_merged: u64,
    sectors_read: u64,
    ms_reading: u64,
    writes: u64,
    writes_merged: u64,
    sectors_written: u64,
    ms_writing: u64,
    ms_doing_io: u64,
    ms_weighted_io: u64,
    discards: u64,
    discard_sectors: u64,
    flushes: u64,
}

/// Parses /proc/diskstats lines keyed by device name. The 11 core fields
/// are mandatory; discard/flush fields (kernel 4.18+/5.5+) default to zero
/// when absent so older kernels still produce I/O stats.
fn diskstats_from(raw: &str) -> HashMap<String, RawCounters> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let mut f = line.split_whitespace();
        let (_major, _minor, name) = (f.next(), f.next(), f.next());
        let Some(name) = name else { continue };
        let mut n = [0u64; DISKSTAT_FIELDS];
        let mut count = 0usize;
        for slot in n.iter_mut() {
            match f.next().and_then(|v| v.parse().ok()) {
                Some(v) => {
                    *slot = v;
                    count += 1;
                }
                None => break,
            }
        }
        if count < DISKSTAT_MIN_FIELDS {
            continue;
        }
        out.insert(
            name.to_string(),
            RawCounters {
                reads: n[0],
                reads_merged: n[1],
                sectors_read: n[2],
                ms_reading: n[3],
                writes: n[4],
                writes_merged: n[5],
                sectors_written: n[6],
                ms_writing: n[7],
                ms_doing_io: n[9],
                ms_weighted_io: n[10],
                discards: n[11],
                discard_sectors: n[13],
                flushes: n[15],
            },
        );
    }
    out
}

/// Deltas between two diskstats samples -> iostat-style metrics.
fn io_stats_from(prev: &RawCounters, cur: &RawCounters, elapsed_secs: f32) -> DiskIoStats {
    let e = if elapsed_secs <= 0.0 {
        1.0
    } else {
        elapsed_secs
    };
    let d = |a: u64, b: u64| b.saturating_sub(a);
    let reads = d(prev.reads, cur.reads);
    let writes = d(prev.writes, cur.writes);
    let r_await = if reads > 0 {
        d(prev.ms_reading, cur.ms_reading) as f32 / reads as f32
    } else {
        0.0
    };
    let w_await = if writes > 0 {
        d(prev.ms_writing, cur.ms_writing) as f32 / writes as f32
    } else {
        0.0
    };
    let r_merge = reads + d(prev.reads_merged, cur.reads_merged);
    let w_merge = writes + d(prev.writes_merged, cur.writes_merged);
    DiskIoStats {
        r_s: rate(reads, e),
        w_s: rate(writes, e),
        r_await_ms: r_await,
        w_await_ms: w_await,
        queue_avg: d(prev.ms_weighted_io, cur.ms_weighted_io) as f32 / (e * MS_PER_SEC),
        busy_pct: d(prev.ms_doing_io, cur.ms_doing_io) as f32 / (e * MS_PER_SEC / 100.0),
        read_merge_pct: if r_merge > 0 {
            d(prev.reads_merged, cur.reads_merged) as f32 / r_merge as f32 * 100.0
        } else {
            0.0
        },
        write_merge_pct: if w_merge > 0 {
            d(prev.writes_merged, cur.writes_merged) as f32 / w_merge as f32 * 100.0
        } else {
            0.0
        },
        read_req_kib: if reads > 0 {
            d(prev.sectors_read, cur.sectors_read) as f32 * BYTES_PER_SECTOR as f32
                / BYTES_PER_KIB as f32
                / reads as f32
        } else {
            0.0
        },
        write_req_kib: if writes > 0 {
            d(prev.sectors_written, cur.sectors_written) as f32 * BYTES_PER_SECTOR as f32
                / BYTES_PER_KIB as f32
                / writes as f32
        } else {
            0.0
        },
        d_s: rate(d(prev.discards, cur.discards), e),
        discard_req_kib: {
            let discards = d(prev.discards, cur.discards);
            if discards > 0 {
                d(prev.discard_sectors, cur.discard_sectors) as f32 * BYTES_PER_SECTOR as f32
                    / 1024.0
                    / discards as f32
            } else {
                0.0
            }
        },
        flush_s: rate(d(prev.flushes, cur.flushes), e),
    }
}

/// Ring-buffer depth for per-disk rate sparklines (~2 min at 1s refresh).
pub const HISTORY_SAMPLES: usize = 120;

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

impl Default for DiskMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskMonitor {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
            last_refresh: None,
            prev_stats: HashMap::new(),
            io_stats: HashMap::new(),
            usage_rates: HashMap::new(),
            io_pressure: [0.0; 3],
            dm_aliases: HashMap::new(),
            history: HashMap::new(),
        }
    }

    pub fn refresh(&mut self) {
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
        self.io_stats = cur
            .iter()
            .filter_map(|(name, c)| {
                self.prev_stats
                    .get(name)
                    .map(|p| (name.clone(), io_stats_from(p, c, elapsed)))
            })
            .collect();
        // Byte-rate history for the sparklines, from the same diskstats
        // deltas as the table columns (not sysinfo, which lags a refresh).
        for (name, c) in cur.iter() {
            if let Some(p) = self.prev_stats.get(name) {
                let rb = d_sectors(p.sectors_read, c.sectors_read) * BYTES_PER_SECTOR;
                let wb = d_sectors(p.sectors_written, c.sectors_written) * BYTES_PER_SECTOR;
                let (rq, wq) = self
                    .history
                    .entry(name.clone())
                    .or_insert_with(|| (VecDeque::new(), VecDeque::new()));
                push_capped(rq, rb as f32 / elapsed.max(0.001), HISTORY_SAMPLES);
                push_capped(wq, wb as f32 / elapsed.max(0.001), HISTORY_SAMPLES);
            }
        }
        self.history.retain(|name, _| cur.contains_key(name));
        self.prev_stats = cur;
        self.last_refresh = Some(now);
        let (p10, p60, p300) = psi::some("io");
        self.io_pressure = [p10, p60, p300];
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
                    temp_c: nvme_temp_c(
                        &Path::new("/sys/block")
                            .join(self.dm_aliases.get(key.as_str()).unwrap_or(&key)),
                    ),
                    io,
                }
            })
            .collect()
    }
}

/// Sector delta between two diskstats samples.
fn d_sectors(a: u64, b: u64) -> u64 {
    b.saturating_sub(a)
}

/// Push into a capped ring buffer.
fn push_capped(q: &mut VecDeque<f32>, v: f32, cap: usize) {
    q.push_back(v);
    if q.len() > cap {
        q.pop_front();
    }
}

/// First hwmon temp1_input (millidegrees) under `dir`, preferring the NVMe
/// controller (hwmon "name" == "nvme"); other sensors are a fallback since
/// some block devices expose unrelated hwmon temps.
fn scan_hwmon(dir: &Path) -> Option<f32> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut fallback: Option<f32> = None;
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with("hwmon") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(e.path().join("temp1_input")) else {
            continue;
        };
        let Ok(milli) = raw.trim().parse::<i64>() else {
            continue;
        };
        let is_nvme = std::fs::read_to_string(e.path().join("name"))
            .map(|n| n.trim() == "nvme")
            .unwrap_or(false);
        if is_nvme {
            return Some(milli as f32 / MS_PER_SEC);
        }
        if fallback.is_none() {
            fallback = Some(milli as f32 / MS_PER_SEC);
        }
    }
    fallback
}

/// Device temperature in °C from the first hwmon temp1_input of the disk
/// node. Candidates in order: the node itself, its `device` symlink (which
/// on sysfs points at the controller that owns hwmon), the parent disk node
/// for partitions, and each dm slave plus its parent.
fn nvme_temp_c(block_dir: &Path) -> Option<f32> {
    let name = block_dir.file_name()?.to_str()?;
    let mut nodes: Vec<std::path::PathBuf> =
        vec![block_dir.to_path_buf(), block_dir.join("device")];
    if !block_dir.join("device").is_dir() {
        if let Some(p) = name.rfind('p') {
            nodes.push(block_dir.parent()?.join(&name[..p]));
        } else {
            for slave in std::fs::read_dir(block_dir.join("slaves")).ok()?.flatten() {
                // slaves/ entries are symlinks to the real disk node.
                let real = std::fs::canonicalize(slave.path()).ok()?;
                nodes.push(real.clone());
                if let Some(parent) = real.parent() {
                    nodes.push(parent.to_path_buf());
                }
            }
        }
    }
    for node in nodes {
        if let Some(t) = scan_hwmon(&node.join("device")).or_else(|| scan_hwmon(&node)) {
            return Some(t);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> &'static str {
        "259 0 nvme0n1 1000 0 100000 500 200 0 40000 400 0 600 800 50 0 2000 0 30\n"
    }

    #[test]
    fn rate_converts_delta_to_bps() {
        assert_eq!(rate(1_000_000, 2.0), 500_000);
        assert_eq!(rate(0, 2.0), 0);
        assert_eq!(rate(500, 0.0), 0);
    }

    #[test]
    fn diskstats_parses_counters() {
        let m = diskstats_from(line());
        let c = &m["nvme0n1"];
        assert_eq!(c.reads, 1000);
        assert_eq!(c.sectors_read, 100000);
        assert_eq!(c.ms_reading, 500);
        assert_eq!(c.writes, 200);
        assert_eq!(c.ms_doing_io, 600);
        assert_eq!(c.ms_weighted_io, 800);
        assert_eq!(c.discards, 50);
        assert_eq!(c.discard_sectors, 2000);
        assert_eq!(c.flushes, 30);
    }

    #[test]
    fn diskstats_skips_garbage() {
        let m = diskstats_from("garbage line\n259 0 nvme0n1 1 2 3 4 5\n");
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn diskstats_old_kernel_without_discard_fields() {
        // Pre-4.18 kernels emit only the 11 core fields; discards/flushes
        // must default to zero instead of dropping the whole device.
        let raw = "259 0 nvme0n1 1000 0 100000 500 200 0 40000 400 0 600 800\n";
        let m = diskstats_from(raw);
        let c = &m["nvme0n1"];
        assert_eq!(c.reads, 1000);
        assert_eq!(c.discards, 0);
        assert_eq!(c.flushes, 0);
    }

    #[test]
    fn scan_hwmon_prefers_nvme_name() {
        let base = std::env::temp_dir().join(format!("perfo-hwmon-name-{}", std::process::id()));
        // Unknown sensor first, nvme second: must pick the nvme one.
        std::fs::create_dir_all(base.join("hwmon0")).unwrap();
        std::fs::write(base.join("hwmon0/temp1_input"), "99000\n").unwrap();
        std::fs::write(base.join("hwmon0/name"), "acpitz\n").unwrap();
        std::fs::create_dir_all(base.join("hwmon1")).unwrap();
        std::fs::write(base.join("hwmon1/temp1_input"), "45000\n").unwrap();
        std::fs::write(base.join("hwmon1/name"), "nvme\n").unwrap();
        assert_eq!(scan_hwmon(&base), Some(45.0));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn io_stats_computes_discard_and_flush() {
        let prev = diskstats_from(line());
        // +100 discards, +8000 sectors, +200 flushes in 2s.
        let cur_raw =
            "259 0 nvme0n1 3000 0 300000 900 400 0 80000 800 0 2600 2800 150 0 10000 0 230\n";
        let cur = diskstats_from(cur_raw);
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 2.0);
        assert_eq!(s.d_s, 50);
        assert_eq!(s.flush_s, 100);
        // 8000 sectors * 512 / 1024 / 100 discards = 40 KiB
        assert_eq!(s.discard_req_kib, 40.0);
    }

    #[test]
    fn push_capped_keeps_newest() {
        let mut q = VecDeque::new();
        for i in 0..5 {
            push_capped(&mut q, i as f32, 3);
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.iter().copied().collect::<Vec<_>>(), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn nvme_temp_reads_milli_degrees() {
        let base = std::env::temp_dir().join(format!("perfo-hwmon-test-{}", std::process::id()));
        std::fs::create_dir_all(base.join("device/hwmon0")).unwrap();
        std::fs::write(base.join("device/hwmon0/temp1_input"), "47800\n").unwrap();
        assert_eq!(nvme_temp_c(&base), Some(47.8));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn nvme_temp_partition_uses_parent_device() {
        let base = std::env::temp_dir().join(format!("perfo-hwmon-part-{}", std::process::id()));
        std::fs::create_dir_all(base.join("nvme0n1/device/hwmon0")).unwrap();
        std::fs::write(base.join("nvme0n1/device/hwmon0/temp1_input"), "40000\n").unwrap();
        // nvme0n1p1 has no /device; must walk up to nvme0n1.
        assert_eq!(nvme_temp_c(&base.join("nvme0n1p1")), Some(40.0));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn nvme_temp_dm_resolves_through_slaves() {
        let base = std::env::temp_dir().join(format!("perfo-hwmon-dm-{}", std::process::id()));
        // sysfs layout: partition dir lives INSIDE the disk node dir.
        std::fs::create_dir_all(base.join("nvme0n1/nvme0n1p2")).unwrap();
        std::fs::create_dir_all(base.join("nvme0n1/device/hwmon0")).unwrap();
        std::fs::write(base.join("nvme0n1/device/hwmon0/temp1_input"), "41000\n").unwrap();
        std::fs::create_dir_all(base.join("dm-0/slaves")).unwrap();
        std::os::unix::fs::symlink(
            base.join("nvme0n1/nvme0n1p2"),
            base.join("dm-0/slaves/nvme0n1p2"),
        )
        .unwrap();
        // dm-0 has no /device and no 'p'; must resolve via slaves/.
        assert_eq!(nvme_temp_c(&base.join("dm-0")), Some(41.0));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn nvme_temp_missing_dir_is_none() {
        assert_eq!(nvme_temp_c(Path::new("/nonexistent/block")), None);
    }

    #[test]
    fn io_stats_computes_await_and_busy() {
        let prev = diskstats_from(line());
        // 2s later: 2000 reads done, 400ms total reading, 2000ms doing io.
        let cur_raw =
            "259 0 nvme0n1 3000 0 300000 900 400 0 80000 800 0 2600 2800 50 0 2000 0 30\n";
        let cur = diskstats_from(cur_raw);
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 2.0);
        assert_eq!(s.r_s, 1000);
        assert_eq!(s.w_s, 100);
        // (900-500)ms / 2000 reads = 0.2ms
        assert_eq!(s.r_await_ms, 0.2);
        // (2600-600)ms / (2s*1000) = 1.0
        assert_eq!(s.queue_avg, 1.0);
        // (2800-800)ms / (2s*10) = 100%
        assert_eq!(s.busy_pct, 100.0);
    }

    #[test]
    fn io_stats_zero_elapsed_is_safe() {
        let prev = diskstats_from(line());
        let cur = diskstats_from(line());
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 0.0);
        assert_eq!(s.r_s, 0);
        assert_eq!(s.busy_pct, 0.0);
    }

    #[test]
    fn io_stats_computes_merge_and_req_size() {
        let prev = diskstats_from(line());
        let cur_raw =
            "259 0 nvme0n1 2000 1000 200000 600 300 150 50000 500 0 700 900 50 0 2000 0 30\n";
        let cur = diskstats_from(cur_raw);
        let s = io_stats_from(&prev["nvme0n1"], &cur["nvme0n1"], 1.0);
        // 1000 extra merged of 2000 total (1000 reads + 1000 merged) -> 50%
        assert!((s.read_merge_pct - 50.0).abs() < 0.01);
        // 100000 extra sectors * 512 / 1024 / 1000 reads = 50 KiB
        assert_eq!(s.read_req_kib, 50.0);
        // (500-400)ms / 100 writes = 1ms
        assert_eq!(s.w_await_ms, 1.0);
    }
}

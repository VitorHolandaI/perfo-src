//! Filesystem usage and block-device I/O collection.

mod monitor;
mod parse;
mod temp;

pub use monitor::DiskMonitor;

use serde::Serialize;

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
    /// NOT saturation: a drive with 16 queues reports 100% with one
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

/// Ring-buffer depth for per-disk rate sparklines (~2 min at 1s refresh).
pub const HISTORY_SAMPLES: usize = 120;

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;

    use super::monitor::push_capped;
    use super::parse::{diskstats_from, io_stats_from};

    use super::temp::*;
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

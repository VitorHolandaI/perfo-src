//! Turning /proc/diskstats counters into per-device I/O rates.

use std::collections::HashMap;

use super::{rate, DiskIoStats};

/// Bytes in one diskstats sector (kernel convention, independent of the
/// device's logical block size).
pub(super) const BYTES_PER_SECTOR: u64 = 512;

/// KiB in bytes.
const BYTES_PER_KIB: u64 = 1024;

/// Milliseconds per second.
pub(super) const MS_PER_SEC: f32 = 1000.0;

/// /proc/diskstats fields read into RawCounters (17 = discard + flush data).
const DISKSTAT_FIELDS: usize = 17;

/// Minimum fields a line must have (pre-4.18 kernels lack discard/flush).
const DISKSTAT_MIN_FIELDS: usize = 11;

/// Raw per-device counters from /proc/diskstats (kernel iostats fields).
#[derive(Clone, Default)]
pub(super) struct RawCounters {
    pub(super) reads: u64,
    pub(super) reads_merged: u64,
    pub(super) sectors_read: u64,
    pub(super) ms_reading: u64,
    pub(super) writes: u64,
    pub(super) writes_merged: u64,
    pub(super) sectors_written: u64,
    pub(super) ms_writing: u64,
    pub(super) ms_doing_io: u64,
    pub(super) ms_weighted_io: u64,
    pub(super) discards: u64,
    pub(super) discard_sectors: u64,
    pub(super) flushes: u64,
}

/// Parses /proc/diskstats lines keyed by device name. The 11 core fields
/// are mandatory; discard/flush fields (kernel 4.18+/5.5+) default to zero
/// when absent so older kernels still produce I/O stats.
pub(super) fn diskstats_from(raw: &str) -> HashMap<String, RawCounters> {
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
pub(super) fn io_stats_from(
    prev: &RawCounters,
    cur: &RawCounters,
    elapsed_secs: f32,
) -> DiskIoStats {
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

/// Sector delta between two diskstats samples.
pub(super) fn d_sectors(a: u64, b: u64) -> u64 {
    b.saturating_sub(a)
}

use serde::Serialize;

use super::psi;

#[derive(Serialize)]
pub struct MemSnapshot {
    pub total: u64,
    /// Memory used by applications: total - free - buffers - cache.
    pub used: u64,
    /// Page cache + reclaimable slab (minus shmem).
    pub cache: u64,
    pub buffers: u64,
    pub free: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    /// PSI memory pressure "some" averages (10s/60s/300s).
    pub psi_some_10: f64,
    pub psi_some_60: f64,
    pub psi_some_300: f64,
}

/// Parses /proc/meminfo into a map of kB values converted to bytes.
fn meminfo_from(raw: &str) -> std::collections::HashMap<String, u64> {
    let mut map = std::collections::HashMap::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
            let key = key.trim_end_matches(':');
            if let Ok(v) = val.parse::<u64>() {
                map.insert(key.to_string(), v * 1024);
            }
        }
    }
    map
}

fn meminfo() -> std::collections::HashMap<String, u64> {
    std::fs::read_to_string("/proc/meminfo")
        .map(|raw| meminfo_from(&raw))
        .unwrap_or_default()
}

/// PSI "some" averages (10s/60s/300s) from /proc/pressure/memory.
fn psi_some() -> (f64, f64, f64) {
    psi::some("memory")
}

pub fn snapshot() -> MemSnapshot {
    let m = meminfo();
    let total = m.get("MemTotal").copied().unwrap_or(0);
    let free = m.get("MemFree").copied().unwrap_or(0);
    let buffers = m.get("Buffers").copied().unwrap_or(0);
    let shmem = m.get("Shmem").copied().unwrap_or(0);
    let cache = m
        .get("Cached")
        .copied()
        .unwrap_or(0)
        .saturating_add(m.get("SReclaimable").copied().unwrap_or(0))
        .saturating_sub(shmem);
    let used = total
        .saturating_sub(free)
        .saturating_sub(buffers)
        .saturating_sub(cache);
    let swap_total = m.get("SwapTotal").copied().unwrap_or(0);
    let swap_free = m.get("SwapFree").copied().unwrap_or(0);
    let (p10, p60, p300) = psi_some();
    MemSnapshot {
        total,
        used,
        cache,
        buffers,
        free,
        available: m.get("MemAvailable").copied().unwrap_or(0),
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
        psi_some_10: p10,
        psi_some_60: p60,
        psi_some_300: p300,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_converts_kb_to_bytes() {
        let raw = "MemTotal:       16000000 kB\nMemFree:         2000000 kB\nBuffers:          100000 kB\n";
        let m = meminfo_from(raw);
        assert_eq!(m.get("MemTotal"), Some(&16_384_000_000));
        assert_eq!(m.get("MemFree"), Some(&2_048_000_000));
    }

    #[test]
    fn meminfo_skips_garbage_lines() {
        let raw = "MemTotal:       16000000 kB\nnot a meminfo line\nBuffers:  nope kB\n";
        let m = meminfo_from(raw);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("MemTotal"), Some(&16_384_000_000));
    }
}

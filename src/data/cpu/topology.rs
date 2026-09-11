//! Static CPU topology: core counts, hybrid P/E classification, per-core
//! frequency ceilings and temperatures.

use std::collections::HashMap;

/// CPUID leaf 0x1A reports hybrid core types; below that the CPU has none.
const CPUID_HYBRID_LEAF: u32 = 0x1A;
/// The core-type byte lives in the low 8 bits of eax.
const CORE_TYPE_MASK: u32 = 0xFF;

use sysinfo::Components;

use serde::Serialize;

/// cpuinfo_max_freq is exposed in kHz; our numbers are MHz.
pub(crate) const KHZ_PER_MHZ: u64 = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
pub enum CoreType {
    /// Performance core: private L2.
    P,
    /// Efficient core: shares L2 with a cluster.
    E,
    /// Low-power efficient core: shared L2, much lower max frequency.
    Lpe,
    /// Reserved for hardware shapes the heuristics cannot classify; serialized
    /// to JSON so the widget can render it instead of crashing.
    #[allow(dead_code)]
    Unknown,
}

impl CoreType {
    pub fn letter(self) -> char {
        match self {
            CoreType::P => 'P',
            CoreType::E => 'E',
            CoreType::Lpe => 'L',
            CoreType::Unknown => '?',
        }
    }
}

pub(crate) fn cpu_count() -> usize {
    std::fs::read_dir("/sys/devices/system/cpu")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    n.starts_with("cpu") && n[3..].chars().all(|c| c.is_ascii_digit())
                })
                .count()
        })
        .unwrap_or(0)
}

/// Parses a cpumask-style "a,b-d" list into individual cpu indexes.
pub(crate) fn shared_l2_from_list(raw: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in raw.trim().split(',') {
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = a.trim().parse().unwrap_or(0);
            let b: usize = b.trim().parse().unwrap_or(a);
            for c in a..=b {
                out.push(c);
            }
        } else if let Ok(c) = part.trim().parse::<usize>() {
            out.push(c);
        }
    }
    out
}

/// CPUs sharing this core's L2 (reads the kernel's shared_cpu_list).
pub(crate) fn shared_l2_cpus(i: usize) -> Vec<usize> {
    let path = format!("/sys/devices/system/cpu/cpu{i}/cache/index2/shared_cpu_list");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    shared_l2_from_list(&raw)
}

/// core_id of cpu N (for mapping coretemp "Core N" labels to cpu indexes).
pub(crate) fn core_id_of(cpu: usize) -> Option<u32> {
    let path = format!("/sys/devices/system/cpu/cpu{cpu}/topology/core_id");
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

/// Per-cpu temperature from coretemp/hwmon components, matched by core_id.
pub(crate) fn per_core_temps(components: &Components) -> Vec<Option<f32>> {
    let mut by_core: HashMap<u32, f32> = HashMap::new();
    for c in components.list() {
        let label = c.label();
        // sysinfo prefixes labels with the hwmon name, e.g. "coretemp Core 0".
        let short = label.strip_prefix("coretemp ").unwrap_or(label);
        if let Some(id) = short
            .strip_prefix("Core")
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            if let Some(t) = c.temperature() {
                by_core.insert(id, t);
            }
        }
    }
    (0..cpu_count())
        .map(|i| core_id_of(i).and_then(|id| by_core.get(&id).copied()))
        .collect()
}

pub(crate) fn cpu_temperature(components: &Components) -> Option<f32> {
    components
        .list()
        .iter()
        .filter(|component| {
            let label = component.label();
            let short = label.strip_prefix("coretemp ").unwrap_or(label);
            short.contains("Package") || short.starts_with("Core") || short.contains("PECI")
        })
        .filter_map(|component| component.temperature())
        .max_by(|left, right| left.total_cmp(right))
}

/// Max frequency of each cpu (MHz) from cpufreq sysfs (source is kHz).
pub(crate) fn per_core_max_freqs() -> Vec<u64> {
    (0..cpu_count())
        .map(|i| {
            std::fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{i}/cpufreq/cpuinfo_max_freq"
            ))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|khz| khz / KHZ_PER_MHZ)
            .unwrap_or(0)
        })
        .collect()
}

/// Hybrid-core classification from the shared-L2 topology only: private L2
/// => P-core, shared L2 => E-core. Used when no frequency data exists.
pub(crate) fn core_type_of(shares_l2_with_others: bool, _max_freq_mhz: u64) -> CoreType {
    if shares_l2_with_others {
        CoreType::E
    } else {
        CoreType::P
    }
}

/// Distinct per-core max frequencies, descending (e.g. [4900, 4400, 2500]).
pub(crate) fn freq_buckets(max_freqs: &[u64]) -> Vec<u64> {
    let mut v: Vec<u64> = max_freqs.iter().copied().filter(|&f| f > 0).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    v
}

/// Intel hybrid core type via CPUID leaf 0x1A (std intrinsic, zero deps).
///
/// Encoding CHANGED between generations, so this is only a best-effort
/// fallback when cpufreq is missing: Alder Lake reports 1=Atom(E)/2=Core(P);
/// Arrow Lake reports 2=Atom(LPE)/3=Core(E+P). The frequency buckets are
/// the authoritative source.
pub(crate) fn hybrid_core_type() -> Option<CoreType> {
    #[cfg(target_arch = "x86_64")]
    {
        let v = std::arch::x86_64::__cpuid(0);
        let ebx = v.ebx.to_ne_bytes();
        let edx = v.edx.to_ne_bytes();
        let ecx = v.ecx.to_ne_bytes();
        if ebx != *b"Genu" || edx != *b"ineI" || ecx != *b"ntel" {
            return None;
        }
        if v.eax < CPUID_HYBRID_LEAF {
            return None;
        }
        let out = std::arch::x86_64::__cpuid_count(CPUID_HYBRID_LEAF, 0);
        match out.eax & CORE_TYPE_MASK {
            // Alder Lake: Atom = E-core.
            1 => Some(CoreType::E),
            // Alder Lake: Core = P-core (Arrow Lake: Atom = LPE).
            2 => Some(CoreType::P),
            // Arrow Lake: Core/Efficient = P and E cores alike.
            3 => Some(CoreType::E),
            _ => None,
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

/// Runs `f` pinned to the given CPU (sched_setaffinity, libc: the binary
/// already links it for ptrace). Returns None when pinning fails.
pub(crate) fn on_cpu<F: FnOnce() -> Option<CoreType> + Send>(cpu: usize, f: F) -> Option<CoreType> {
    std::thread::scope(|s| {
        let handle = s.spawn(move || {
            let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
            // SAFETY: set is zeroed and sized by libc; CPU_SET is the
            // documented macro, sched_setaffinity(0) targets this thread.
            unsafe {
                libc::CPU_ZERO(&mut set);
                libc::CPU_SET(cpu, &mut set);
            }
            let rc =
                unsafe { libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) };
            if rc == 0 {
                f()
            } else {
                None
            }
        });
        handle.join().ok().flatten()
    })
}

/// P/E/LPE per core, generation-independent:
/// 1. Distinct max-freq buckets: the top bucket is P, the bottom is LPE
///    (3 buckets), the middle E. Works on Alder Lake and Arrow Lake alike
///    (their cpuid 0x1A encodings differ).
/// 2. Uniform max freq: not hybrid (shared-L2 topology decides P vs E).
/// 3. No cpufreq at all: best-effort cpuid 0x1A probe per core.
pub(crate) fn core_types_of(max_freqs: &[u64]) -> Vec<CoreType> {
    let buckets = freq_buckets(max_freqs);
    if buckets.len() >= 2 {
        let p_max = buckets[0];
        let lpe_max = buckets.get(2).copied();
        max_freqs
            .iter()
            .map(|&f| {
                if Some(f) == lpe_max && buckets.len() >= 3 {
                    CoreType::Lpe
                } else if f == p_max {
                    CoreType::P
                } else {
                    CoreType::E
                }
            })
            .collect()
    } else if buckets.len() == 1 {
        (0..cpu_count())
            .map(|i| {
                let others = shared_l2_cpus(i).iter().filter(|&&c| c != i).count();
                core_type_of(others > 0, 0)
            })
            .collect()
    } else {
        (0..cpu_count())
            .map(|i| {
                on_cpu(i, hybrid_core_type).unwrap_or_else(|| {
                    let others = shared_l2_cpus(i).iter().filter(|&&c| c != i).count();
                    core_type_of(others > 0, 0)
                })
            })
            .collect()
    }
}

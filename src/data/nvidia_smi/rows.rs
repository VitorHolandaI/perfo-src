//! Parsing of the CSV that `nvidia-smi --format=csv,noheader,nounits` prints,
//! kept apart from the process that produces it.
//!
//! Every column is optional: a card answers `[N/A]` or `[Not Supported]` for a
//! sensor it does not have, and a driver error arrives on the same stream as
//! data, so a row that does not parse is dropped rather than guessed at.

use super::super::gpu::GpuProcessInfo;
use crate::units::clamp_percent;

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
}

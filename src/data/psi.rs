/// PSI (Pressure Stall Information) parsing, shared by memory and I/O.
///
/// /proc/pressure/<resource> lines look like:
///   some avg10=1.23 avg60=0.45 avg300=0.10 total=12345
///   full avg10=...
/// We only consume the "some" line (wall-clock time stalled).
use std::fs;

/// "some" averages (10s/60s/300s) from a /proc/pressure file body.
pub fn some_from(raw: &str) -> (f64, f64, f64) {
    let mut out = (0.0, 0.0, 0.0);
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("some ") {
            let mut vals = [0.0; 3];
            let mut i = 0;
            for field in rest.split_whitespace() {
                if let Some(v) = field
                    .strip_prefix("avg10=")
                    .or_else(|| field.strip_prefix("avg60="))
                    .or_else(|| field.strip_prefix("avg300="))
                {
                    if i < 3 {
                        vals[i] = v.parse().unwrap_or(0.0);
                        i += 1;
                    }
                }
            }
            out = (vals[0], vals[1], vals[2]);
            break;
        }
    }
    out
}

/// "some" averages for a given pressure file (/proc/pressure/<name>).
pub fn some(name: &str) -> (f64, f64, f64) {
    fs::read_to_string(format!("/proc/pressure/{name}"))
        .map(|raw| some_from(&raw))
        .unwrap_or((0.0, 0.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psi_parses_some_line_only() {
        let raw = "some avg10=1.23 avg60=0.45 avg300=0.10 total=12345\nfull avg10=9.99 avg60=8.88 avg300=7.77 total=6789\n";
        assert_eq!(some_from(raw), (1.23, 0.45, 0.10));
    }

    #[test]
    fn psi_missing_file_defaults_zero() {
        assert_eq!(some_from(""), (0.0, 0.0, 0.0));
    }

    #[test]
    fn psi_ignores_full_only_files() {
        assert_eq!(
            some_from("full avg10=9.99 avg60=8.88 avg300=7.77 total=1\n"),
            (0.0, 0.0, 0.0)
        );
    }
}

//! Conversions between the raw counters the collectors read and the
//! percentages the UI renders.

/// `part` as a percentage of `whole`. A zero `whole` reads as 0 rather than
/// NaN, which would otherwise reach a label or a bar width.
pub fn percent_of(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 as f32 / whole as f64 as f32 * 100.0
}

/// Clamps a percentage reported by hardware into 0..=100. Drivers occasionally
/// report values outside that range, and a few report NaN.
pub fn clamp_percent(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// How many cells of `width` a percentage fills, rounded to the nearest cell.
pub fn fill_width(percent: f32, width: u16) -> u16 {
    let filled = (clamp_percent(percent) / 100.0 * width as f32).round();
    filled as u16
}

/// Splits a duration into whole minutes and the leftover seconds, which is how
/// every elapsed/total label in the UI is written.
pub fn minutes_seconds(seconds: u64) -> (u64, u64) {
    (seconds / SECONDS_PER_MINUTE, seconds % SECONDS_PER_MINUTE)
}

/// PSI is reported over three windows: 10s, 60s and 300s.
pub const PSI_WINDOWS: usize = 3;
/// Floor for an elapsed-seconds divisor, so a zero interval cannot divide.
pub const MIN_ELAPSED_SECS: f32 = 0.001;
/// Divisor between adjacent binary size units.
pub const BYTES_PER_KIB: f32 = 1024.0;
/// Microwatts per watt, as amdgpu reports power.
pub const MICRO_PER_UNIT: f64 = 1_000_000.0;

pub const SECONDS_PER_MINUTE: u64 = 60;
pub const SECONDS_PER_HOUR: u64 = 3600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_seconds_splits_a_duration() {
        assert_eq!(minutes_seconds(0), (0, 0));
        assert_eq!(minutes_seconds(59), (0, 59));
        assert_eq!(minutes_seconds(60), (1, 0));
        assert_eq!(minutes_seconds(3661), (61, 1));
    }

    #[test]
    fn percent_of_guards_a_zero_whole() {
        assert_eq!(percent_of(0, 0), 0.0);
        assert_eq!(percent_of(5, 0), 0.0);
        assert_eq!(percent_of(1, 4), 25.0);
        assert_eq!(percent_of(4, 4), 100.0);
    }

    #[test]
    fn clamp_percent_rejects_out_of_range_and_nan() {
        assert_eq!(clamp_percent(50.0), 50.0);
        assert_eq!(clamp_percent(-3.0), 0.0);
        assert_eq!(clamp_percent(140.0), 100.0);
        assert_eq!(clamp_percent(f32::NAN), 0.0);
        assert_eq!(clamp_percent(f32::INFINITY), 0.0);
    }

    #[test]
    fn fill_width_spans_the_whole_bar() {
        assert_eq!(fill_width(0.0, 10), 0);
        assert_eq!(fill_width(50.0, 10), 5);
        assert_eq!(fill_width(100.0, 10), 10);
        assert_eq!(fill_width(f32::NAN, 10), 0);
        assert_eq!(fill_width(200.0, 10), 10);
    }
}

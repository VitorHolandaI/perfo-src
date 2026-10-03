//! How long a session recording runs before it stops and saves itself.
//!
//! The recorder counts samples, not wall time, and the TUI samples once per
//! `TICK` (1s), so a duration in minutes becomes `minutes * 60` samples.

use super::HistoryState;

/// Minutes the picker offers before "custom". Used to be a fixed 120s.
pub const RECORD_MINUTE_PRESETS: [usize; 4] = [2, 5, 10, 40];

/// Ceiling on a typed duration. The whole recording sits in memory until it
/// is saved, and at the default depth a sample is ~27 KB (30 processes at
/// ~905 B each), so two hours is already ~200 MB.
pub const MAX_CUSTOM_MINUTES: usize = 120;

/// Three digits is enough to type the ceiling and no more.
const MAX_CUSTOM_DIGITS: usize = 3;

/// One sample per second, see the module docs.
const SAMPLES_PER_MINUTE: usize = 60;

/// Which duration the picker has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordDuration {
    /// Index into `RECORD_MINUTE_PRESETS`.
    Preset(usize),
    /// Minutes typed into `HistoryState::record_custom_minutes`.
    Custom,
}

impl Default for RecordDuration {
    fn default() -> Self {
        RecordDuration::Preset(0)
    }
}

/// How the selection reads in the picker, e.g. `5 min` or `custom: 15_ min`.
///
/// ```ignore
/// assert_eq!(record_duration_label(RecordDuration::Preset(1), ""), "5 min");
/// ```
pub fn record_duration_label(choice: RecordDuration, custom_minutes: &str) -> String {
    match choice {
        RecordDuration::Preset(i) => format!("{} min", RECORD_MINUTE_PRESETS[i]),
        RecordDuration::Custom => format!("custom: {}_ min", custom_minutes),
    }
}

impl HistoryState {
    /// Steps through the presets and then "custom", wrapping both ways.
    pub fn cycle_record_duration(&mut self, forward: bool) {
        let slots = RECORD_MINUTE_PRESETS.len() + 1;
        let current = match self.record_duration {
            RecordDuration::Preset(i) => i,
            RecordDuration::Custom => RECORD_MINUTE_PRESETS.len(),
        };
        let next = if forward {
            (current + 1) % slots
        } else {
            (current + slots - 1) % slots
        };
        self.record_duration = if next == RECORD_MINUTE_PRESETS.len() {
            RecordDuration::Custom
        } else {
            RecordDuration::Preset(next)
        };
    }

    /// Appends a typed digit to the custom minutes. Non-digits and digits past
    /// the length cap are ignored, since the field only ever holds a number.
    pub fn push_custom_minutes_digit(&mut self, c: char) {
        if !c.is_ascii_digit() || self.record_custom_minutes.len() >= MAX_CUSTOM_DIGITS {
            return;
        }
        self.record_custom_minutes.push(c);
    }

    pub fn pop_custom_minutes_digit(&mut self) {
        self.record_custom_minutes.pop();
    }

    /// Samples the chosen duration stops at, or why it cannot start.
    ///
    /// ```ignore
    /// state.record_duration = RecordDuration::Preset(0);
    /// assert_eq!(state.chosen_record_samples(), Ok(120));
    /// ```
    pub fn chosen_record_samples(&self) -> Result<usize, String> {
        let minutes = match self.record_duration {
            RecordDuration::Preset(i) => RECORD_MINUTE_PRESETS[i],
            RecordDuration::Custom => parse_custom_minutes(&self.record_custom_minutes)?,
        };
        Ok(minutes * SAMPLES_PER_MINUTE)
    }
}

/// Validates the typed minutes: a whole number from 1 to `MAX_CUSTOM_MINUTES`.
fn parse_custom_minutes(typed: &str) -> Result<usize, String> {
    let minutes: usize = typed.parse().map_err(|_| {
        format!(
            "custom duration '{}' is not a number: type 1-{} minutes",
            typed, MAX_CUSTOM_MINUTES
        )
    })?;
    if minutes == 0 || minutes > MAX_CUSTOM_MINUTES {
        return Err(format!(
            "custom duration {} min is out of range: expected 1-{} minutes",
            minutes, MAX_CUSTOM_MINUTES
        ));
    }
    Ok(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_cycles_through_presets_then_custom_and_wraps() {
        let mut state = HistoryState::default();
        let mut seen = Vec::new();
        for _ in 0..=RECORD_MINUTE_PRESETS.len() {
            seen.push(state.record_duration);
            state.cycle_record_duration(true);
        }
        assert_eq!(
            seen,
            vec![
                RecordDuration::Preset(0),
                RecordDuration::Preset(1),
                RecordDuration::Preset(2),
                RecordDuration::Preset(3),
                RecordDuration::Custom,
            ]
        );
        assert_eq!(state.record_duration, RecordDuration::Preset(0));

        state.cycle_record_duration(false);
        assert_eq!(state.record_duration, RecordDuration::Custom);
    }

    /// The old fixed 120s stays the default, so nothing changes for someone
    /// who never touches the row.
    #[test]
    fn default_duration_is_two_minutes() {
        let state = HistoryState::default();
        assert_eq!(state.chosen_record_samples(), Ok(120));
    }

    #[test]
    fn presets_convert_minutes_to_one_sample_per_second() {
        let mut state = HistoryState::default();
        let samples: Vec<usize> = (0..RECORD_MINUTE_PRESETS.len())
            .map(|i| {
                state.record_duration = RecordDuration::Preset(i);
                state.chosen_record_samples().unwrap()
            })
            .collect();
        assert_eq!(samples, vec![120, 300, 600, 2400]);
    }

    #[test]
    fn custom_minutes_take_digits_only_up_to_three() {
        let mut state = HistoryState::default();
        for c in ['1', 'x', '5', ' ', '0', '9'] {
            state.push_custom_minutes_digit(c);
        }
        assert_eq!(state.record_custom_minutes, "150");
        state.pop_custom_minutes_digit();
        assert_eq!(state.record_custom_minutes, "15");
    }

    #[test]
    fn custom_duration_uses_the_typed_minutes() {
        let state = HistoryState {
            record_duration: RecordDuration::Custom,
            record_custom_minutes: "15".to_string(),
            ..HistoryState::default()
        };
        assert_eq!(state.chosen_record_samples(), Ok(900));
    }

    #[test]
    fn custom_duration_rejects_empty_zero_and_above_the_ceiling() {
        for typed in ["", "0", "121"] {
            let state = HistoryState {
                record_duration: RecordDuration::Custom,
                record_custom_minutes: typed.to_string(),
                ..HistoryState::default()
            };
            let err = state.chosen_record_samples().unwrap_err();
            assert!(err.contains("1-120"), "{typed:?} gave {err}");
        }
    }

    #[test]
    fn label_shows_minutes_or_the_custom_field() {
        assert_eq!(
            record_duration_label(RecordDuration::Preset(3), ""),
            "40 min"
        );
        assert_eq!(
            record_duration_label(RecordDuration::Custom, "15"),
            "custom: 15_ min"
        );
    }
}

//! Moving the cursor along the timeline, and playback.

use super::{HistorySample, HistorySpan, HistoryState};

impl HistoryState {
    pub fn sample_count(&self) -> usize {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.len()
        } else {
            self.samples.len()
        }
    }

    pub fn get_sample(&self, idx: usize) -> Option<&HistorySample> {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.get(idx)
        } else {
            self.samples.get(idx)
        }
    }

    pub fn last_sample(&self) -> Option<&HistorySample> {
        if self.loaded_session_id.is_some() {
            self.loaded_samples.last()
        } else {
            self.samples.back()
        }
    }

    pub fn effective_index(&self) -> usize {
        let count = self.sample_count();
        if count == 0 {
            return 0;
        }
        match self.scrub_index {
            Some(idx) => idx.min(count - 1),
            None => count - 1,
        }
    }

    pub fn is_live(&self) -> bool {
        self.loaded_session_id.is_none()
            && (self.scrub_index.is_none()
                || self.effective_index() == self.samples.len().saturating_sub(1))
    }

    pub fn step(&mut self, delta: i32) {
        let count = self.sample_count();
        if count == 0 {
            return;
        }
        self.playing = false;
        let eff = self.effective_index();
        let target = eff as i64 + delta as i64;
        if target < 0 {
            self.scrub_index = Some(0);
        } else if target >= (count - 1) as i64 {
            if self.loaded_session_id.is_some() {
                self.scrub_index = Some(count - 1);
            } else {
                self.scrub_index = None;
            }
        } else {
            self.scrub_index = Some(target as usize);
        }
    }

    pub fn jump_step(&self) -> usize {
        match self.span {
            HistorySpan::Span2m => 10,
            HistorySpan::Span15m => 30,
            HistorySpan::Span1h => 60,
            HistorySpan::SpanAll => 300,
        }
    }

    pub fn jump(&mut self, direction: i32) {
        let step = self.jump_step() as i32 * (if direction < 0 { -1 } else { 1 });
        self.step(step);
    }

    pub fn toggle_playback(&mut self) {
        let count = self.sample_count();
        if count == 0 {
            return;
        }
        if self.playing {
            self.playing = false;
        } else {
            if self.is_live() && count > 0 {
                let span = self.span.seconds(count);
                self.scrub_index = Some(count.saturating_sub(span));
            }
            self.playing = true;
        }
    }

    pub fn advance_playback(&mut self) {
        let count = self.sample_count();
        if !self.playing || count == 0 {
            return;
        }
        let eff = self.effective_index();
        if eff + 1 >= count {
            if self.loaded_session_id.is_some() {
                self.scrub_index = Some(count - 1);
            } else {
                self.scrub_index = None;
            }
            self.playing = false;
        } else {
            self.scrub_index = Some(eff + 1);
        }
    }

    pub fn jump_to_live(&mut self) {
        self.loaded_session_id = None;
        self.loaded_session_title = None;
        self.loaded_samples.clear();
        self.scrub_index = None;
        self.playing = false;
    }
}

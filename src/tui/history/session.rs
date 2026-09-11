//! Starting, stopping and loading saved recordings.

use std::time::Instant;

use super::HistorySample;
use super::HistoryState;

impl HistoryState {
    pub fn stop_and_save_session(&mut self) {
        if !self.is_session_recording || self.session_record_buffer.is_empty() {
            self.is_session_recording = false;
            self.session_record_buffer.clear();
            return;
        }
        self.is_session_recording = false;
        let dur = self.session_record_buffer.len() as u64;
        let val = serde_json::to_value(&self.session_record_buffer)
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        let focus = self.recording_mask.summary();
        match crate::recordings::save_session_data(val, dur, &focus) {
            Ok(meta) => {
                let rec_id = meta.id.clone();
                let dur_str = meta.duration.clone();
                let _ = self.load_session(&rec_id);
                if !self.loaded_samples.is_empty() {
                    self.scrub_index = Some(self.loaded_samples.len() - 1);
                }
                self.export_status = Some((
                    format!("Saved session: {} ({}) [{}]", rec_id, dur_str, focus),
                    Instant::now(),
                ));
            }
            Err(e) => {
                self.export_status = Some((format!("Failed to save: {}", e), Instant::now()));
            }
        }
        self.session_record_buffer.clear();
    }

    pub fn open_record_modal(&mut self) {
        self.record_modal = true;
        self.record_modal_idx = 0;
    }

    pub fn close_record_modal(&mut self) {
        self.record_modal = false;
    }

    pub fn record_modal_next(&mut self) {
        self.record_modal_idx = (self.record_modal_idx + 1) % 8;
    }

    pub fn record_modal_prev(&mut self) {
        self.record_modal_idx = if self.record_modal_idx == 0 {
            7
        } else {
            self.record_modal_idx - 1
        };
    }

    pub fn toggle_record_mask_item(&mut self, idx: usize) {
        let count = self.recording_mask.count();
        let target = match idx {
            0 => &mut self.recording_mask.cpu,
            1 => &mut self.recording_mask.mem,
            2 => &mut self.recording_mask.io,
            3 => &mut self.recording_mask.net,
            4 => &mut self.recording_mask.gpu,
            5 => &mut self.recording_mask.npu,
            _ => return,
        };
        if !*target || count > 1 {
            *target = !*target;
        }
    }

    pub fn start_session_recording(&mut self) {
        self.record_modal = false;
        self.is_session_recording = true;
        self.session_record_buffer.clear();
        let summary = self.recording_mask.summary();
        self.export_status = Some((
            format!("Recording session [{}] started...", summary),
            Instant::now(),
        ));
    }

    pub fn toggle_session_recording(&mut self) {
        if self.is_session_recording {
            self.stop_and_save_session();
        } else {
            self.open_record_modal();
        }
    }

    pub fn open_sessions_modal(&mut self) {
        self.refresh_saved_recordings();
        self.selected_session_idx = 0;
        self.sessions_modal = true;
    }

    pub fn refresh_saved_recordings(&mut self) {
        self.saved_recordings = crate::recordings::get_recordings_list();
        if self.saved_recordings.is_empty() {
            self.selected_session_idx = 0;
        } else if self.selected_session_idx >= self.saved_recordings.len() {
            self.selected_session_idx = self.saved_recordings.len() - 1;
        }
    }

    pub fn close_sessions_modal(&mut self) {
        self.sessions_modal = false;
    }

    pub fn modal_next(&mut self) {
        if !self.saved_recordings.is_empty() {
            self.selected_session_idx =
                (self.selected_session_idx + 1).min(self.saved_recordings.len() - 1);
        }
    }

    pub fn modal_prev(&mut self) {
        if self.selected_session_idx > 0 {
            self.selected_session_idx -= 1;
        }
    }

    pub fn modal_load_selected(&mut self) {
        if let Some(meta) = self.saved_recordings.get(self.selected_session_idx) {
            let id = meta.id.clone();
            if let Err(e) = self.load_session(&id) {
                self.export_status = Some((format!("Load error: {}", e), Instant::now()));
            }
        }
    }

    pub fn modal_delete_selected(&mut self) {
        if let Some(meta) = self.saved_recordings.get(self.selected_session_idx) {
            let id = meta.id.clone();
            let _ = crate::recordings::delete_recording_by_id(&id);
            self.export_status = Some((format!("Deleted session {}", id), Instant::now()));
            self.refresh_saved_recordings();
        }
    }

    pub fn load_session(&mut self, id: &str) -> Result<(), String> {
        let payload = crate::recordings::load_recording_payload(id).map_err(|e| e.to_string())?;
        let samples: Vec<HistorySample> = serde_json::from_value(payload.samples)
            .map_err(|e| format!("Failed to parse samples: {}", e))?;
        if samples.is_empty() {
            return Err("Session has no samples".to_string());
        }
        let title = format!("{} ({})", payload.id, payload.duration_label);
        self.loaded_session_id = Some(payload.id.clone());
        self.loaded_session_title = Some(title);
        self.loaded_samples = samples;
        self.scrub_index = Some(0);
        self.playing = false;
        self.sessions_modal = false;
        self.export_status = Some((format!("Loaded replay {}", id), Instant::now()));
        Ok(())
    }
}

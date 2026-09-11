//! Session recordings: the flight recorder's on-disk format and CLI.

mod export;
mod format;
mod paths;
mod store;
mod timeline;

pub use export::export_from_stdin;
pub use format::format_duration;
pub use paths::{
    default_recordings_dir, get_config, get_home_dir, resolve_recording_path,
    DEFAULT_MAX_RECORDINGS,
};
pub use store::{
    delete_recording, delete_recording_by_id, get_recording, get_recordings_list, list_recordings,
    load_recording_payload, prune_recordings, save_recording, save_session_data,
};
pub use timeline::{get_timeline, inspect_recording, load_offsets, scan_sample_offsets};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;

#[derive(Serialize, Deserialize, Debug)]
pub struct RecordingMetadata {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub date: String,
    pub time: String,
    pub duration: String,
    pub duration_seconds: u64,
    pub sample_count: usize,
    pub metric_focus: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RecordingPayload {
    pub id: String,
    pub filename: String,
    pub date: String,
    pub time: String,
    pub duration_seconds: u64,
    pub duration_label: String,
    pub metric_focus: String,
    pub sample_count: usize,
    pub samples: Value,
}

pub fn dispatch(subcmd: &str, args: &[String]) -> io::Result<()> {
    match subcmd {
        "list" => list_recordings(),
        "save" => save_recording(args.first().map(|s| s.as_str())),
        "get" => get_recording(args.first().map(|s| s.as_str())),
        "delete" => delete_recording(args.first().map(|s| s.as_str())),
        "timeline" => {
            let target = args.first().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Missing recording path or ID")
            })?;
            get_timeline(target)
        }
        "inspect" => {
            let target = args.first().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Missing recording path or ID")
            })?;
            let idx = args
                .get(1)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            let win = args
                .get(2)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(15);
            inspect_recording(target, idx, win)
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unknown record subcommand: {}", other),
        )),
    }
}

//! Writing a history report to the user's home directory.

use serde_json::Value;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use super::paths::get_home_dir;

pub fn export_from_stdin(basename: &str) -> io::Result<(String, String)> {
    let mut buf = String::new();
    let n = io::stdin().read_line(&mut buf)?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Empty export input",
        ));
    }
    let trimmed = buf.trim();
    let raw = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        buf
    } else {
        let _ = io::stdin().read_to_string(&mut buf);
        buf
    };
    if raw.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Empty export input",
        ));
    }
    let val: Value = serde_json::from_str(raw.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let home = get_home_dir();
    let txt_name = format!("{}.txt", basename);
    let json_name = format!("{}.json", basename);
    let txt_path = Path::new(&home).join(&txt_name);
    let json_path = Path::new(&home).join(&json_name);

    if let Some(txt) = val.get("text").and_then(|v| v.as_str()) {
        fs::write(&txt_path, txt)?;
    }
    if let Some(json_val) = val.get("json") {
        let json_str = match json_val {
            Value::String(s) => s.clone(),
            _ => serde_json::to_string_pretty(json_val).map_err(io::Error::other)?,
        };
        fs::write(&json_path, json_str)?;
    }

    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "Perfo",
            "History Exported",
            &format!("Saved: ~/{}.{{txt,json}}", basename),
        ])
        .spawn();

    Ok((
        txt_path.to_string_lossy().to_string(),
        json_path.to_string_lossy().to_string(),
    ))
}

//! Copying a full command line out of the TUI.

pub(super) fn to_base64(data: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(B64[(triple >> 18) & 0x3F] as char);
        result.push(B64[(triple >> 12) & 0x3F] as char);
        if chunk.len() > 1 {
            result.push(B64[(triple >> 6) & 0x3F] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(B64[triple & 0x3F] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub(super) fn copy_to_clipboard(text: &str) {
    let mut done = false;
    // 1. Try wl-copy (Wayland)
    if let Ok(mut child) = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait() {
            if status.success() {
                done = true;
            }
        }
    }

    // 2. Try xclip (X11) if not already copied
    if !done {
        if let Ok(mut child) = std::process::Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    done = true;
                }
            }
        }
    }

    // 3. Try xsel (X11) if not already copied
    if !done {
        if let Ok(mut child) = std::process::Command::new("xsel")
            .arg("-b")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }

    // 4. Always emit OSC 52 sequence to stdout (works across modern terminals & SSH)
    let b64 = to_base64(text.as_bytes());
    use std::io::Write;
    let _ = std::io::stdout().write_all(format!("\x1b]52;c;{b64}\x07").as_bytes());
    let _ = std::io::stdout().flush();
}

pub(super) fn get_process_full_cmd(pid: u32) -> String {
    if let Ok(content) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        if !content.is_empty() {
            let mut parts = Vec::new();
            for part in content.split(|&b| b == 0) {
                if !part.is_empty() {
                    parts.push(String::from_utf8_lossy(part).to_string());
                }
            }
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
    }
    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        let trimmed = comm.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format!("[{pid}]")
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::tui::State;

    use crate::tui::keys::handle_key;
    use crossterm::event::{KeyCode, KeyModifiers};
    #[test]
    fn base64_encoding_works() {
        assert_eq!(to_base64(b""), "");
        assert_eq!(to_base64(b"f"), "Zg==");
        assert_eq!(to_base64(b"fo"), "Zm8=");
        assert_eq!(to_base64(b"foo"), "Zm9v");
        assert_eq!(to_base64(b"perfo monitor"), "cGVyZm8gbW9uaXRvcg==");
    }

    #[test]
    fn copy_key_sets_status_message() {
        let mut s = State {
            selected_pid: Some(std::process::id()),
            fullscreen: true,
            ..State::default()
        };
        handle_key(&mut s, &[], KeyCode::Char('y'), KeyModifiers::empty(), None);
        assert!(s.status_msg.is_some());
        assert!(
            s.status_msg.as_ref().unwrap().contains("copiado")
                || s.status_msg.as_ref().unwrap().contains("Copied")
        );
    }
}

//! Colours, units and the small text helpers every pane shares.

use std::collections::VecDeque;

use ratatui::{
    style::{Color, Style},
    widgets::{Block, Borders},
};

use crate::theme::Theme;

/// Core/frequency color thresholds (percentages).
pub(crate) const HOT_PCT: f32 = 80.0;

pub(crate) const WARN_PCT: f32 = 50.0;

/// Disk bar thresholds (percentages).
pub(crate) const DISK_HOT_PCT: f32 = 85.0;

pub(crate) const DISK_WARN_PCT: f32 = 70.0;

/// Frequency ratio thresholds (of the core's own max).
pub(crate) const FREQ_HIGH_RATIO: f32 = 0.66;

pub(crate) const FREQ_MID_RATIO: f32 = 0.33;

pub(crate) fn cpu_color(v: f32, theme: &Theme) -> Color {
    if v >= HOT_PCT {
        theme.red
    } else if v >= WARN_PCT {
        theme.yellow
    } else {
        theme.green
    }
}

pub(crate) fn bar(value: f32, width: usize) -> String {
    let filled = crate::units::fill_width(value, width as u16) as usize;
    let filled = filled.min(width);
    format!(
        "{}{}",
        bar_glyph(value).to_string().repeat(filled),
        "·".repeat(width - filled)
    )
}

pub(crate) fn bar_glyph(value: f32) -> char {
    const LEVELS: [char; 9] = ['.', ':', '-', '=', '+', '*', '#', '%', '@'];
    let level = (value.clamp(0.0, 100.0) / 100.0 * (LEVELS.len() - 1) as f32).round() as usize;
    LEVELS[level]
}

pub(crate) fn ghz(f: u64) -> String {
    if f >= 1000 {
        format!("{:.1}G", f as f32 / 1000.0)
    } else {
        format!("{f}M")
    }
}

/// GHz relative to the core's own max frequency: red near the limit,
/// yellow mid, green comfortably below.
pub(crate) fn freq_color(freq: u64, max: u64, theme: &Theme) -> Color {
    if max == 0 {
        return theme.muted;
    }
    let ratio = freq as f32 / max as f32;
    if ratio >= FREQ_HIGH_RATIO {
        theme.red
    } else if ratio >= FREQ_MID_RATIO {
        theme.yellow
    } else {
        theme.green
    }
}

pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1}{}", UNITS[unit])
}

/// Compact size: 8.3G, 535M, 20K.
pub(crate) fn short_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "K", "M", "G"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Shorten to AT MOST `max` chars (ellipsis included), so callers can use
/// it inside fixed-width table cells without breaking alignment.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Shorten to AT MOST `max` chars (ellipsis included), starting at character `offset`.
pub(crate) fn truncate_with_scroll(s: &str, offset: usize, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let total_chars = s.chars().count();
    if offset >= total_chars {
        return String::new();
    }
    let skipped: String = s.chars().skip(offset).collect();
    if skipped.chars().count() <= max {
        skipped
    } else {
        let mut t: String = skipped.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Disks with distinct device names, in mount order, capped at 5. btrfs
/// subvolumes share one device and would otherwise render as duplicate bars.
pub(crate) fn unique_disks(
    disks: &[crate::data::disk::DiskInfo],
) -> Vec<&crate::data::disk::DiskInfo> {
    let mut seen = std::collections::HashSet::new();
    disks
        .iter()
        .filter(|d| seen.insert(d.name.clone()))
        .take(5)
        .collect()
}

/// Trend graph: samples bucketed to `width` columns, scaled to the
/// absolute 0-100 range. Oldest left, newest right. Right-padded when
/// fewer samples than width (graph grows from the left).
pub(crate) fn sparkline(samples: &VecDeque<f32>, width: usize, fixed_max: Option<f32>) -> String {
    const CHARS: [char; 9] = ['⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷', '⣿', '⣿'];
    if width == 0 {
        return String::new();
    }
    if samples.is_empty() {
        return " ".repeat(width);
    }
    let bucket_n = samples.len().div_ceil(width).max(1);
    let mut vals: Vec<f32> = Vec::new();
    let mut sum = 0.0;
    let mut cnt = 0usize;
    for (i, v) in samples.iter().enumerate() {
        sum += v;
        cnt += 1;
        if (i + 1) % bucket_n == 0 || i == samples.len() - 1 {
            vals.push(sum / cnt as f32);
            sum = 0.0;
            cnt = 0;
        }
    }
    let scale = fixed_max
        .unwrap_or_else(|| vals.iter().copied().fold(0.0, f32::max))
        .max(0.001);
    let line: String = vals
        .iter()
        .map(|v| CHARS[((v / scale * 8.0) as usize).min(8)])
        .collect();
    if vals.len() < width {
        format!("{:<width$}", line, width = width)
    } else {
        line
    }
}

/// NVMe/SATA temperature color: green <55°C, yellow 55-70, red >70
/// (drives throttle around 80°C).
pub(crate) fn temp_color(temp_c: Option<f32>, t: &Theme) -> Color {
    match temp_c {
        Some(c) if c >= 70.0 => t.red,
        Some(c) if c >= 55.0 => t.yellow,
        _ => t.green,
    }
}

/// Latency color: green < 2ms, yellow < 10ms, red >= 10ms (NVMe health
/// line sits below 1ms; 10ms+ means the queue is backing up).
pub(crate) fn await_color(ms: f32, theme: &Theme) -> Color {
    if ms >= 10.0 {
        theme.red
    } else if ms >= 2.0 {
        theme.yellow
    } else {
        theme.green
    }
}

/// Queue depth: green < 2, yellow < 8, red >= 8.
pub(crate) fn queue_color(q: f32, theme: &Theme) -> Color {
    if q >= 8.0 {
        theme.red
    } else if q >= 2.0 {
        theme.yellow
    } else {
        theme.green
    }
}

/// Busy%: red only past 90 (remember: on NVMe this is not saturation).
pub(crate) fn busy_color(pct: f32, theme: &Theme) -> Color {
    if pct >= 90.0 {
        theme.yellow
    } else {
        theme.muted
    }
}

pub(crate) fn io_pressure_color(p10: f64, theme: &Theme) -> Color {
    if p10 >= 10.0 {
        theme.red
    } else if p10 >= 5.0 {
        theme.yellow
    } else {
        theme.green
    }
}

pub(crate) fn block(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    let mut b = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL);
    if focused {
        b = b.border_style(Style::default().fg(theme.accent));
    }
    b
}

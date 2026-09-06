use std::collections::VecDeque;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row as TableRow, Table, TableState},
    Frame,
};

use crate::data::cpu::{CoreType, CpuSnapshot, ProcessInfo};
use crate::theme::Theme;

/// Core/frequency color thresholds (percentages).
const HOT_PCT: f32 = 80.0;
const WARN_PCT: f32 = 50.0;
/// Frequency ratio thresholds (of the core's own max).
const FREQ_HIGH_RATIO: f32 = 0.66;
const FREQ_MID_RATIO: f32 = 0.33;
/// Disk bar thresholds (percentages).
const DISK_HOT_PCT: f32 = 85.0;
const DISK_WARN_PCT: f32 = 70.0;
/// Cap on rendered core rows (defensive against huge machines).
const MAX_CORE_ROWS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Cpu,
    Mem,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Cpu,
    Io,
    Net,
    Mem,
    Disks,
    Gpu,
}

pub struct Row<'a> {
    pub depth: usize,
    pub process: &'a ProcessInfo,
}

pub struct Ui<'a> {
    pub snap: &'a CpuSnapshot,
    pub rows: &'a [Row<'a>],
    pub selected: Option<usize>,
    pub core_focus: usize,
    pub core_filter: Option<usize>,
    pub sort: SortKey,
    pub invert: bool,
    pub full_cmd: bool,
    pub tree: bool,
    pub pane: Pane,
    pub fullscreen: bool,
    pub theme: Theme,
    pub help: bool,
    pub help_page: usize,
    pub show_menu: bool,
    pub cores_focused: bool,
    pub lang: crate::tui::Lang,
    pub tracing: bool,
    pub trace_lines: Option<&'a std::collections::VecDeque<String>>,
    pub trace_pid: Option<u32>,
    pub status: &'a str,
    pub searching: bool,
    pub kill_prompt: bool,
}

fn cpu_color(v: f32, theme: &Theme) -> Color {
    if v >= HOT_PCT {
        theme.red
    } else if v >= WARN_PCT {
        theme.yellow
    } else {
        theme.green
    }
}

pub(super) fn bar(value: f32, width: usize) -> String {
    let filled = ((value.clamp(0.0, 100.0) / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!(
        "{}{}",
        bar_glyph(value).to_string().repeat(filled),
        "·".repeat(width - filled)
    )
}

pub(super) fn bar_glyph(value: f32) -> char {
    const LEVELS: [char; 9] = ['.', ':', '-', '=', '+', '*', '#', '%', '@'];
    let level = (value.clamp(0.0, 100.0) / 100.0 * (LEVELS.len() - 1) as f32).round() as usize;
    LEVELS[level]
}

fn ghz(f: u64) -> String {
    if f >= 1000 {
        format!("{:.1}G", f as f32 / 1000.0)
    } else {
        format!("{f}M")
    }
}

/// GHz relative to the core's own max frequency: red near the limit,
/// yellow mid, green comfortably below.
fn freq_color(freq: u64, max: u64, theme: &Theme) -> Color {
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

pub(super) fn human_bytes(bytes: u64) -> String {
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
pub(super) fn short_bytes(bytes: u64) -> String {
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
pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Disks with distinct device names, in mount order, capped at 5. btrfs
/// subvolumes share one device and would otherwise render as duplicate bars.
pub(super) fn unique_disks(
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
pub(super) fn sparkline(samples: &VecDeque<f32>, width: usize, fixed_max: Option<f32>) -> String {
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
pub(super) fn temp_color(temp_c: Option<f32>, t: &Theme) -> Color {
    match temp_c {
        Some(c) if c >= 70.0 => t.red,
        Some(c) if c >= 55.0 => t.yellow,
        _ => t.green,
    }
}

pub fn draw(frame: &mut Frame, ui: &Ui) {
    if ui.fullscreen {
        // Focused window owns the whole terminal (status line + help still
        // overlay on top).
        let [body, status_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        match ui.pane {
            Pane::Cpu => draw_cpu_pane(frame, body, ui),
            Pane::Io => draw_io(frame, body, ui),
            Pane::Net => draw_net(frame, body, ui),
            Pane::Mem => super::detail::draw_mem(frame, body, ui),
            Pane::Disks => super::detail::draw_disks(frame, body, ui),
            Pane::Gpu => super::detail::draw_gpu(frame, body, ui),
        }
        draw_status(frame, status_area, ui);
    } else {
        // Dashboard keeps one compact summary of every subsystem visible;
        // `m` or the number shortcuts open a detailed view.
        let core_lines = ui.snap.per_core.len().min(MAX_CORE_ROWS).div_ceil(2);
        let [cpu_area, mid_area, lower_area, status_area] = Layout::vertical([
            Constraint::Length(6 + core_lines as u16),
            Constraint::Length(7),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        draw_cpu(frame, cpu_area, ui, true);
        let [mem_area, disk_area, io_area] = Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ])
        .areas(mid_area);
        draw_mem(frame, mem_area, ui);
        draw_disks(frame, disk_area, ui);
        draw_io_summary(frame, io_area, ui);
        let [net_area, proc_area] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(lower_area);
        draw_net_summary(frame, net_area, ui);
        draw_process_summary(frame, proc_area, ui);
        draw_status(frame, status_area, ui);
    }
    if ui.help {
        draw_help(frame, frame.area(), ui);
    }
    if ui.show_menu {
        draw_menu(frame, frame.area(), ui);
    }
}

/// Fullscreen CPU window: one frame around CPU, memory, disks, and processes.
fn draw_cpu_pane(frame: &mut Frame, area: Rect, ui: &Ui) {
    let core_lines = ui.snap.per_core.len().min(MAX_CORE_ROWS).div_ceil(2);
    let title = match ui.core_filter {
        Some(c) => format!("1:CPU + PROCESSES — core {c}"),
        None => "1:CPU + PROCESSES".to_string(),
    };
    let outer = block(&title, true, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);
    let [cpu_area, mid_area, proc_area] = Layout::vertical([
        Constraint::Length(6 + core_lines as u16),
        Constraint::Length(7),
        Constraint::Min(0),
    ])
    .areas(inner);
    draw_cpu(frame, cpu_area, ui, false);
    let [mem_area, disk_area, gpu_area] = Layout::horizontal([
        Constraint::Percentage(32),
        Constraint::Percentage(43),
        Constraint::Percentage(25),
    ])
    .areas(mid_area);
    draw_mem(frame, mem_area, ui);
    draw_disks(frame, disk_area, ui);
    super::detail::draw_gpu_summary(frame, gpu_area, ui);
    draw_processes(frame, proc_area, ui, false);
}

fn draw_summary(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, theme: &Theme) {
    let panel = block(title, false, theme);
    frame.render_widget(panel.clone(), area);
    frame.render_widget(Paragraph::new(lines), panel.inner(area));
}

fn draw_io_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    let disks = unique_disks(&ui.snap.disks);
    let read: u64 = disks.iter().map(|d| d.read_bps).sum();
    let write: u64 = disks.iter().map(|d| d.write_bps).sum();
    let lines = vec![
        Line::from(format!("read  {:>8}/s", short_bytes(read))),
        Line::from(format!("write {:>8}/s", short_bytes(write))),
        Line::from(format!(
            "pressure {:.1}  busy {:.0}%",
            ui.snap.io_pressure_some[0],
            disks.iter().map(|d| d.io.busy_pct).sum::<f32>()
        )),
        Line::from(format!("disks {}", disks.len())),
    ];
    draw_summary(frame, area, "2:IO", lines, &ui.theme);
}

fn draw_net_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    super::net_summary::draw(frame, area, ui);
}

fn draw_process_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    let lines: Vec<Line> = ui
        .rows
        .iter()
        .take(area.height.saturating_sub(1) as usize)
        .map(|row| {
            Line::from(format!(
                "{:>6} {:>4.1}% {}",
                row.process.pid,
                row.process.cpu_percent,
                truncate(&row.process.cmd, area.width.saturating_sub(15) as usize)
            ))
        })
        .collect();
    draw_summary(frame, area, "1:PROCS", lines, &ui.theme);
}

pub(super) fn block(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    let mut b = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL);
    if focused {
        b = b.border_style(Style::default().fg(theme.accent));
    }
    b
}

fn draw_cpu(frame: &mut Frame, area: Rect, ui: &Ui, framed: bool) {
    let title = match ui.core_filter {
        Some(c) => format!("1:CPU — core {c}"),
        None => "1:CPU".to_string(),
    };
    let inner = if framed {
        let panel = block(&title, ui.pane == Pane::Cpu, &ui.theme);
        frame.render_widget(panel.clone(), area);
        panel.inner(area)
    } else {
        area
    };
    let [overall_area, cores_area] =
        Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).areas(inner);

    let bar_w = overall_area.width.saturating_sub(26) as usize;
    let color = cpu_color(ui.snap.overall_percent, &ui.theme);
    let overall = Line::from(vec![
        Span::styled("overall ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            bar(ui.snap.overall_percent, bar_w),
            Style::default().fg(color),
        ),
        Span::styled(
            format!(" {:>5.1}%", ui.snap.overall_percent),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]);
    let la = ui.snap.load_avg;
    let temp = match ui.snap.cpu_temp_c {
        Some(t) => format!("    cpu {t:.0}\u{00B0}C"),
        None => String::new(),
    };
    let mem = format!(
        "load {:.2} {:.2} {:.2}    mem {}/{}    iowait {:.1}%",
        la[0],
        la[1],
        la[2],
        human_bytes(ui.snap.used_mem_bytes),
        human_bytes(ui.snap.total_mem_bytes),
        ui.snap.iowait_percent
    );
    let legend = Line::from(vec![
        Span::styled("P ", Style::default().fg(ui.theme.accent)),
        Span::styled("performance   ", Style::default().fg(ui.theme.muted)),
        Span::styled("E ", Style::default().fg(ui.theme.green)),
        Span::styled("efficient   ", Style::default().fg(ui.theme.muted)),
        Span::styled("L ", Style::default().fg(ui.theme.red)),
        Span::styled("low-power", Style::default().fg(ui.theme.muted)),
    ]);
    frame.render_widget(
        Paragraph::new(vec![
            overall,
            Line::from(vec![
                Span::styled("overall history ", Style::default().fg(ui.theme.muted)),
                Span::styled(
                    sparkline(&ui.snap.cpu_history, bar_w.min(100), Some(100.0)),
                    Style::default().fg(cpu_color(ui.snap.overall_percent, &ui.theme)),
                ),
            ]),
            Line::from(Span::raw(format!("{mem}{temp}"))),
            legend,
        ]),
        overall_area,
    );
    draw_cores(frame, cores_area, ui);
}

fn draw_cores(frame: &mut Frame, area: Rect, ui: &Ui) {
    let n = ui.snap.per_core.len().min(MAX_CORE_ROWS);
    let two_per_line = area.width >= 60;
    let bar_w = if two_per_line {
        (area.width.saturating_sub(1) / 2).saturating_sub(26) as usize
    } else {
        area.width.saturating_sub(30) as usize
    };
    let bar_w = bar_w.max(1);
    let mut lines: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < n {
        let mut spans: Vec<Span> = Vec::new();
        for slot in 0..2 {
            if i >= n {
                break;
            }
            let usage = ui.snap.per_core[i];
            let letter = ui
                .snap
                .per_core_types
                .get(i)
                .map(|t| t.letter())
                .unwrap_or('?');
            let focused_core = i == ui.core_focus;
            let focus_marker = if ui.cores_focused && focused_core {
                "▶"
            } else {
                " "
            };
            let mut st = Style::default();
            if focused_core {
                st = st.bg(ui.theme.selection).add_modifier(Modifier::BOLD);
            }
            if two_per_line && slot == 1 {
                spans.push(Span::raw("  "));
            }
            let type_color = match ui.snap.per_core_types.get(i) {
                Some(CoreType::P) => ui.theme.accent,
                Some(CoreType::E) => ui.theme.green,
                Some(CoreType::Lpe) => ui.theme.red,
                _ => ui.theme.muted,
            };
            let cur_mhz = ui.snap.per_core_freq_mhz.get(i).copied().unwrap_or(0);
            let freq = if cur_mhz > 0 {
                ghz(cur_mhz)
            } else {
                " - ".to_string()
            };
            let max = ui.snap.per_core_max_freq_mhz.get(i).copied().unwrap_or(0);
            let freq_c = freq_color(cur_mhz, max, &ui.theme);
            let temp = match ui.snap.per_core_temp_c.get(i).copied().flatten() {
                Some(t) => format!(" {t:.0}\u{00B0}"),
                None => String::new(),
            };
            spans.push(Span::styled(
                format!("{focus_marker}{:>2}{} ", i, letter),
                st.fg(type_color),
            ));
            spans.push(Span::styled(
                bar(usage, bar_w),
                st.fg(cpu_color(usage, &ui.theme)),
            ));
            let value_fg = if focused_core {
                ui.theme.fg
            } else {
                ui.theme.muted
            };
            spans.push(Span::styled(format!(" {:>5.1}%", usage), st.fg(value_fg)));
            spans.push(Span::styled(format!(" {freq}"), st.fg(freq_c)));
            spans.push(Span::styled(temp, st.fg(type_color)));
            i += 1;
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_mem(frame: &mut Frame, area: Rect, ui: &Ui) {
    let focused = ui.pane == Pane::Cpu;
    frame.render_widget(block("4:MEM", focused, &ui.theme), area);
    let inner = block("4:MEM", focused, &ui.theme).inner(area);
    let m = &ui.snap.mem;
    let w = inner.width as usize;
    let bar_w = w.saturating_sub(8);
    let frac = |x: u64| (x as f64 / m.total.max(1) as f64 * bar_w as f64) as usize;

    let used_w = frac(m.used);
    let cache_w = frac(m.cache);
    let buf_w = frac(m.buffers);
    let free_w = bar_w.saturating_sub(used_w + cache_w + buf_w);
    let pct = m.used as f32 / m.total.max(1) as f32 * 100.0;

    let bar_line = Line::from(vec![
        Span::styled(
            bar_glyph(pct).to_string().repeat(used_w),
            Style::default().fg(ui.theme.green),
        ),
        Span::styled(
            bar_glyph(m.cache as f32 / m.total.max(1) as f32 * 100.0)
                .to_string()
                .repeat(cache_w),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            bar_glyph(m.buffers as f32 / m.total.max(1) as f32 * 100.0)
                .to_string()
                .repeat(buf_w),
            Style::default().fg(ui.theme.accent),
        ),
        Span::styled("·".repeat(free_w), Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!(" {:>4.0}%", pct),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(ui.theme.fg),
        ),
    ]);

    let detail = format!(
        "used {}  cache {}  buf {}  free {}",
        short_bytes(m.used),
        short_bytes(m.cache),
        short_bytes(m.buffers),
        short_bytes(m.free)
    );

    let swap_line = if m.swap_total > 0 {
        let swap_used_w = (m.swap_used as f64 / m.swap_total as f64 * bar_w as f64) as usize;
        let swap_pct = m.swap_used as f32 / m.swap_total as f32 * 100.0;
        Line::from(vec![
            Span::styled("swap ", Style::default().fg(ui.theme.muted)),
            Span::styled(
                bar_glyph(swap_pct).to_string().repeat(swap_used_w),
                Style::default().fg(ui.theme.yellow),
            ),
            Span::styled(
                "·".repeat(bar_w.saturating_sub(swap_used_w)),
                Style::default().fg(ui.theme.muted),
            ),
            Span::styled(
                format!(
                    " {:>4.0}% {}/{}",
                    swap_pct,
                    short_bytes(m.swap_used),
                    short_bytes(m.swap_total)
                ),
                Style::default().fg(ui.theme.muted),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "swap: off",
            Style::default().fg(ui.theme.muted),
        ))
    };

    let psi_color = if m.psi_some_10 > 10.0 {
        ui.theme.red
    } else if m.psi_some_10 > 5.0 {
        ui.theme.yellow
    } else {
        ui.theme.green
    };
    let psi_line = Line::from(vec![
        Span::styled("psi ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!(
                "{:.1} {:.1} {:.1}%",
                m.psi_some_10, m.psi_some_60, m.psi_some_300
            ),
            Style::default().fg(psi_color),
        ),
    ]);

    frame.render_widget(
        Paragraph::new(vec![
            bar_line,
            Line::from(Span::raw(detail)),
            swap_line,
            psi_line,
        ]),
        inner,
    );
}

fn draw_disks(frame: &mut Frame, area: Rect, ui: &Ui) {
    frame.render_widget(block("5:DISKS", false, &ui.theme), area);
    let inner = block("5:DISKS", false, &ui.theme).inner(area);
    let mut lines: Vec<Line> = Vec::new();
    let w = inner.width as usize;
    // name(12) + 1 + bar + pct(18) + 2 + mount(12)
    let bar_w = w.saturating_sub(45);
    for d in unique_disks(&ui.snap.disks) {
        let color = if d.percent >= DISK_HOT_PCT {
            ui.theme.red
        } else if d.percent >= DISK_WARN_PCT {
            ui.theme.yellow
        } else {
            ui.theme.green
        };
        let filled = ((d.percent.clamp(0.0, 100.0) / 100.0) * bar_w as f32).round() as usize;
        let filled = filled.min(bar_w);
        let name = d.name.rsplit('/').next().unwrap_or(&d.name);
        let mount = truncate(&d.mount, 12);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:<11} ", truncate(name, 11)),
                Style::default().fg(ui.theme.muted),
            ),
            Span::styled(
                bar_glyph(d.percent).to_string().repeat(filled),
                Style::default().fg(color),
            ),
            Span::styled(
                "·".repeat(bar_w - filled),
                Style::default().fg(ui.theme.muted),
            ),
            Span::styled(
                format!(
                    " {:>3.0}% {:>5}/{}",
                    d.percent,
                    short_bytes(d.used_bytes),
                    short_bytes(d.total_bytes)
                ),
                Style::default().fg(ui.theme.muted),
            ),
            Span::styled(format!("  {mount}"), Style::default().fg(ui.theme.fg)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Full-pane disk I/O view (menu 2 -> IO): PSI pressure, per-disk iostat-style
/// columns, then the top processes actually moving data.
fn draw_io(frame: &mut Frame, area: Rect, ui: &Ui) {
    let focused = ui.pane == Pane::Io;
    let block = block("2:IO", focused, &ui.theme);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    let disks = unique_disks(&ui.snap.disks);
    let total_r: u64 = disks.iter().map(|d| d.read_bps).sum();
    let total_w: u64 = disks.iter().map(|d| d.write_bps).sum();
    let total_d: u64 = disks.iter().map(|d| d.io.d_s).sum();
    let total_fl: u64 = disks.iter().map(|d| d.io.flush_s).sum();

    let pipe = |s: &str| Span::styled(s.to_string(), Style::default().fg(ui.theme.muted));
    let mut lines = vec![Line::from(vec![
        pipe("│"),
        Span::styled(
            format!(
                " io pressure {:>4.1} {:>4.1} {:>4.1}%",
                ui.snap.io_pressure_some[0],
                ui.snap.io_pressure_some[1],
                ui.snap.io_pressure_some[2]
            ),
            Style::default().fg(io_pressure_color(ui.snap.io_pressure_some[0], &ui.theme)),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>15}", format!("read {}/s", short_bytes(total_r))),
            Style::default().fg(ui.theme.accent),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>15}", format!("write {}/s", short_bytes(total_w))),
            Style::default().fg(ui.theme.accent),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>12}", format!("trim {}/s", short_bytes(total_d))),
            Style::default().fg(ui.theme.muted),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>8}", format!("flush {}/s", total_fl)),
            Style::default().fg(ui.theme.muted),
        ),
        pipe("│"),
        Span::styled(" last refresh", Style::default().fg(ui.theme.muted)),
    ])];

    lines.push(Line::from(vec![
        pipe(&format!(" {:<9}", "DISK")),
        pipe("│"),
        pipe(&io_header("r/s", 7)),
        pipe("│"),
        pipe(&io_header("r_awt ms", 8)),
        pipe("│"),
        pipe(&io_header("w/s", 7)),
        pipe("│"),
        pipe(&io_header("w_awt ms", 8)),
        pipe("│"),
        pipe(&io_header("queue", 6)),
        pipe("│"),
        pipe(&io_header("busy", 6)),
        pipe("│"),
        pipe(&io_header("temp", 5)),
        pipe("│"),
        pipe(&format!("{:>20}", "READ")),
        pipe("│"),
        pipe(&format!("{:>20}", "WRITE")),
        pipe("│"),
        pipe(&format!(" {:<8}", "MOUNT")),
    ]));
    for d in disks {
        let mount = truncate(&d.mount, 8);
        let name = truncate(d.name.rsplit('/').next().unwrap_or(&d.name), 9);
        let key = d.name.rsplit('/').next().unwrap_or(&d.name);
        let (rhist, whist) = ui
            .snap
            .io_history
            .get(key)
            .map(|(r, w)| (r.clone(), w.clone()))
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(format!(" {name:<9}"), Style::default().fg(ui.theme.muted)),
            pipe("│"),
            Span::styled(
                format!(" {:>6}", d.io.r_s),
                Style::default().fg(ui.theme.fg),
            ),
            pipe("│"),
            Span::styled(
                format!(" {:>7.1}", d.io.r_await_ms),
                Style::default().fg(await_color(d.io.r_await_ms, &ui.theme)),
            ),
            pipe("│"),
            Span::styled(
                format!(" {:>6}", d.io.w_s),
                Style::default().fg(ui.theme.fg),
            ),
            pipe("│"),
            Span::styled(
                format!(" {:>7.1}", d.io.w_await_ms),
                Style::default().fg(await_color(d.io.w_await_ms, &ui.theme)),
            ),
            pipe("│"),
            Span::styled(
                format!(" {:>5.1}", d.io.queue_avg),
                Style::default().fg(queue_color(d.io.queue_avg, &ui.theme)),
            ),
            pipe("│"),
            Span::styled(
                format!(" {:>4.0}%", d.io.busy_pct),
                Style::default().fg(busy_color(d.io.busy_pct, &ui.theme)),
            ),
            pipe("│"),
            Span::styled(
                d.temp_c
                    .map(|t| format!(" {:>3.0}°", t))
                    .unwrap_or_else(|| "    -".into()),
                Style::default().fg(temp_color(d.temp_c, &ui.theme)),
            ),
            pipe("│"),
            Span::styled(
                format!(
                    " {:<10} {:>6}/s",
                    sparkline(&rhist, 10, None),
                    short_bytes(d.read_bps)
                ),
                Style::default().fg(ui.theme.accent),
            ),
            pipe("│"),
            Span::styled(
                format!(
                    " {:<10} {:>6}/s",
                    sparkline(&whist, 10, None),
                    short_bytes(d.write_bps)
                ),
                Style::default().fg(ui.theme.yellow),
            ),
            pipe("│"),
            Span::styled(format!(" {mount:<8}"), Style::default().fg(ui.theme.fg)),
        ]));
    }

    // Instant per-process I/O (this tick's read/write rates).
    let mut by_rate: Vec<&ProcessInfo> = ui
        .snap
        .processes
        .iter()
        .filter(|p| p.read_bps > 0 || p.write_bps > 0)
        .collect();
    by_rate.sort_by_key(|a| std::cmp::Reverse(a.read_bps + a.write_bps));
    if !by_rate.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "ACTIVE I/O (by process)",
            Style::default().fg(ui.theme.accent),
        )));
        for p in by_rate.iter().take(6) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<11}", truncate(&p.user, 11)),
                    Style::default().fg(ui.theme.muted),
                ),
                Span::styled(
                    format!(
                        "read {:>7}/s  write {:>7}/s  ",
                        short_bytes(p.read_bps),
                        short_bytes(p.write_bps)
                    ),
                    Style::default().fg(ui.theme.fg),
                ),
                Span::styled(
                    truncate(&p.cmd, inner.width.saturating_sub(40) as usize),
                    Style::default().fg(ui.theme.fg),
                ),
            ]));
        }
    }

    // Top processes by storage I/O accumulated in the current window
    // (IO_WINDOW_SECS): "who hammered the disk lately", not just this tick.
    let mut by_io: Vec<&ProcessInfo> = ui
        .snap
        .processes
        .iter()
        .filter(|p| p.win_read_bytes > 0 || p.win_write_bytes > 0)
        .collect();
    by_io.sort_by_key(|a| std::cmp::Reverse(a.win_read_bytes + a.win_write_bytes));
    if !by_io.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "MOST I/O IN WINDOW (300s)",
            Style::default().fg(ui.theme.accent),
        )));
        for p in by_io.iter().take(6) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<11}", truncate(&p.user, 11)),
                    Style::default().fg(ui.theme.muted),
                ),
                Span::styled(
                    format!(
                        "read {:>6}  write {:>6}  ",
                        short_bytes(p.win_read_bytes),
                        short_bytes(p.win_write_bytes)
                    ),
                    Style::default().fg(ui.theme.fg),
                ),
                Span::styled(
                    truncate(&p.cmd, inner.width.saturating_sub(40) as usize),
                    Style::default().fg(ui.theme.fg),
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn io_header(label: &str, width: usize) -> String {
    format!("{label:>width$}")
}

/// Full-pane network view (menu 3 -> NET): per-interface rx/tx rates and
/// packet/error/drop counters plus TCP retransmissions and connections.
///
/// Column cells (fixed): IFACE(10) | RX/s(9) | TX/s(9) | PPS(17) |
/// ERR/DROP(13) | LINK(14). The legend line uses the SAME cell widths so
/// every `│` lines up across the three rows.
fn draw_net(frame: &mut Frame, area: Rect, ui: &Ui) {
    let focused = ui.pane == Pane::Net;
    let block = block("3:NET", focused, &ui.theme);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    let totals = &ui.snap.net.totals;
    let pipe = |s: &str| Span::styled(s.to_string(), Style::default().fg(ui.theme.muted));

    let mut lines = vec![Line::from(vec![
        pipe(&format!(" {:<9}", "NET")),
        pipe("│"),
        Span::styled(
            format!("{:>20}", format!("rx {}/s", short_bytes(totals.rx_bps))),
            Style::default().fg(ui.theme.accent),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>20}", format!("tx {}/s", short_bytes(totals.tx_bps))),
            Style::default().fg(ui.theme.yellow),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>17}", format!("tcp retrans {}/s", totals.tcp_retrans_s)),
            Style::default().fg(if totals.tcp_retrans_s > 0 {
                ui.theme.red
            } else {
                ui.theme.fg
            }),
        ),
        pipe("│"),
        Span::styled(
            format!("{:>13}", format!("{} connections", totals.tcp_established)),
            Style::default().fg(ui.theme.fg),
        ),
        pipe("│"),
        Span::styled(" last refresh", Style::default().fg(ui.theme.muted)),
    ])];
    lines.push(Line::from(vec![
        Span::styled(" SESSION ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("RX {}", human_bytes(totals.session_rx_bytes)),
            Style::default().fg(ui.theme.accent),
        ),
        Span::styled("  ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("TX {}", human_bytes(totals.session_tx_bytes)),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled("  since monitor start", Style::default().fg(ui.theme.muted)),
    ]));
    lines.push(Line::from(vec![
        pipe(&format!(" {:<9}", "IFACE")),
        pipe("│"),
        pipe(&format!("{:>20}", "RX/s")),
        pipe("│"),
        pipe(&format!("{:>20}", "TX/s")),
        pipe("│"),
        pipe(&format!("{:>17}", "rx pps / tx pps")),
        pipe("│"),
        pipe(&format!("{:>13}", "err / drop")),
        pipe("│"),
        pipe(&format!("{:>14}", "LINK")),
    ]));
    for i in &ui.snap.net.ifaces {
        let link = match (i.link_mbps, i.link_up) {
            (Some(m), true) => format!("{m}M up"),
            (Some(m), false) => format!("{m}M down"),
            (None, true) => "up".to_string(),
            (None, false) => "-".to_string(),
        };
        let errs = i.rx_errs_s + i.tx_errs_s;
        let drops = i.rx_drops_s + i.tx_drops_s;
        let bad_color = if errs + drops > 0 {
            ui.theme.red
        } else {
            ui.theme.muted
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<9}", truncate(&i.name, 9)),
                Style::default().fg(ui.theme.muted),
            ),
            pipe("│"),
            Span::styled(
                format!(
                    "{:<10} {:>9}",
                    sparkline(&i.rx_hist, 10, None),
                    format!("{}/s", short_bytes(i.rx_bps))
                ),
                Style::default().fg(ui.theme.accent),
            ),
            pipe("│"),
            Span::styled(
                format!(
                    "{:<10} {:>9}",
                    sparkline(&i.tx_hist, 10, None),
                    format!("{}/s", short_bytes(i.tx_bps))
                ),
                Style::default().fg(ui.theme.yellow),
            ),
            pipe("│"),
            Span::styled(
                format!("{:>17}", format!("rx {} tx {}", i.rx_pps, i.tx_pps)),
                Style::default().fg(ui.theme.fg),
            ),
            pipe("│"),
            Span::styled(
                format!("{:>13}", format!("{} err {} drop", errs, drops)),
                Style::default().fg(bad_color),
            ),
            pipe("│"),
            Span::styled(format!("{:>14}", link), Style::default().fg(ui.theme.fg)),
        ]));
    }

    // Processes with open sockets (own + readable under yama), like `ss -p`.
    if !ui.snap.net.proc_net.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "NETWORK PROCESSES (tcp est | listen | udp)",
            Style::default().fg(ui.theme.accent),
        )));
        for p in ui.snap.net.proc_net.iter().take(8) {
            let cmd = ui
                .snap
                .processes
                .iter()
                .find(|pr| pr.pid == p.pid)
                .map(|pr| pr.cmd.clone())
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(format!("{:>7}", p.pid), Style::default().fg(ui.theme.muted)),
                Span::styled(
                    format!("  tcp {} | {} | udp {}  ", p.tcp_est, p.tcp_listen, p.udp),
                    Style::default().fg(ui.theme.fg),
                ),
                Span::styled(
                    truncate(&cmd, inner.width.saturating_sub(32) as usize),
                    Style::default().fg(ui.theme.fg),
                ),
            ]));
        }
    }

    // Listening ports (only in fullscreen).
    if focused && !ui.snap.net.listening.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "LISTENING PORTS",
            Style::default().fg(ui.theme.accent),
        )));
        for lp in ui.snap.net.listening.iter().take(20) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:>5}", lp.port),
                    Style::default().fg(ui.theme.yellow),
                ),
                Span::styled(
                    format!(" / {:<4}", lp.proto),
                    Style::default().fg(ui.theme.muted),
                ),
                Span::styled(
                    format!(" {:>7}", lp.pid),
                    Style::default().fg(ui.theme.muted),
                ),
                Span::styled(
                    format!(
                        "  {}",
                        truncate(&lp.cmd, inner.width.saturating_sub(27) as usize)
                    ),
                    Style::default().fg(ui.theme.fg),
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Latency color: green < 2ms, yellow < 10ms, red >= 10ms (NVMe health
/// line sits below 1ms; 10ms+ means the queue is backing up).
fn await_color(ms: f32, theme: &Theme) -> Color {
    if ms >= 10.0 {
        theme.red
    } else if ms >= 2.0 {
        theme.yellow
    } else {
        theme.green
    }
}

/// Queue depth: green < 2, yellow < 8, red >= 8.
fn queue_color(q: f32, theme: &Theme) -> Color {
    if q >= 8.0 {
        theme.red
    } else if q >= 2.0 {
        theme.yellow
    } else {
        theme.green
    }
}

/// Busy%: red only past 90 (remember: on NVMe this is not saturation).
fn busy_color(pct: f32, theme: &Theme) -> Color {
    if pct >= 90.0 {
        theme.yellow
    } else {
        theme.muted
    }
}

fn io_pressure_color(p10: f64, theme: &Theme) -> Color {
    if p10 >= 10.0 {
        theme.red
    } else if p10 >= 5.0 {
        theme.yellow
    } else {
        theme.green
    }
}

fn draw_processes(frame: &mut Frame, area: Rect, ui: &Ui, framed: bool) {
    if ui.tracing {
        draw_trace(frame, area, ui);
        return;
    }
    let title = match (ui.tree, ui.core_filter) {
        (true, Some(c)) => format!("PROCS — core {c} (tree)"),
        (false, Some(c)) => format!("PROCS — core {c}"),
        (true, None) => "PROCS (tree)".to_string(),
        (false, None) => "PROCS".to_string(),
    };
    let inner = if framed {
        let panel = block(&title, ui.pane == Pane::Cpu, &ui.theme);
        frame.render_widget(panel.clone(), area);
        panel.inner(area)
    } else {
        area
    };

    let widths = [
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Min(10),
    ];

    let arrow = if ui.invert { "↓" } else { "↑" };
    let cpu_hdr = if ui.sort == SortKey::Cpu {
        format!("CPU%{arrow}")
    } else {
        "CPU%".to_string()
    };
    let mem_hdr = if ui.sort == SortKey::Mem {
        format!("MEM{arrow}")
    } else {
        "MEM".to_string()
    };
    let header = TableRow::new(vec![
        Cell::from("PID"),
        Cell::from("USER"),
        Cell::from(cpu_hdr),
        Cell::from(mem_hdr),
        Cell::from("COMMAND"),
    ])
    .style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(ui.theme.fg),
    );

    let rows: Vec<TableRow> = ui
        .rows
        .iter()
        .map(|r| {
            let indent = if ui.tree {
                format!("{}▸ ", "  ".repeat(r.depth.min(20)))
            } else {
                String::new()
            };
            let cmd = if ui.full_cmd {
                truncate(&r.process.cmd, 200)
            } else {
                truncate(&r.process.cmd, 40)
            };
            TableRow::new(vec![
                Cell::from(r.process.pid.to_string()),
                Cell::from(r.process.user.as_str()),
                Cell::from(format!("{:.1}", r.process.cpu_percent)),
                Cell::from(human_bytes(r.process.mem_bytes)),
                Cell::from(format!("{indent}{cmd}")),
            ])
        })
        .collect();

    let mut ts = TableState::default();
    ts.select(ui.selected);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(ui.theme.selection))
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn draw_trace(frame: &mut Frame, area: Rect, ui: &Ui) {
    let title = match ui
        .trace_pid
        .and_then(|p| ui.snap.processes.iter().find(|x| x.pid == p))
    {
        Some(proc) => format!("TRACE {} {}", proc.pid, truncate(&proc.cmd, 50)),
        None => format!("TRACE {}", ui.trace_pid.unwrap_or(0)),
    };
    let focused = ui.pane == Pane::Cpu;
    frame.render_widget(block(&title, focused, &ui.theme), area);
    let inner = block(&title, focused, &ui.theme).inner(area);
    let lines: Vec<Line> = match ui.trace_lines {
        Some(ls) => ls
            .iter()
            .take(inner.height as usize)
            .map(|l| Line::from(Span::raw(l.clone())))
            .collect(),
        None => vec![Line::from(Span::raw("starting trace..."))],
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_status(frame: &mut Frame, area: Rect, ui: &Ui) {
    let color = if ui.searching {
        ui.theme.yellow
    } else if ui.kill_prompt {
        ui.theme.red
    } else {
        ui.theme.fg
    };
    frame.render_widget(
        Paragraph::new(ui.status.to_string()).style(Style::default().fg(color)),
        area,
    );
}

fn draw_help(frame: &mut Frame, area: Rect, ui: &Ui) {
    let (title, text) = super::help::page(ui.help_page, ui.lang, &ui.theme);
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(format!(" {title} "));
    frame.render_widget(Clear, area);
    frame.render_widget(block.clone(), area);
    frame.render_widget(Paragraph::new(text), block.inner(area));
}

fn draw_menu(frame: &mut Frame, area: Rect, ui: &Ui) {
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };
    let items: Vec<(&str, &str)> = vec![
        ("1", "CPU + processes"),
        ("2", "I/O (disks)"),
        ("3", "Network"),
        ("4", "Memory"),
        ("5", "Disks"),
        ("6", "GPU"),
        ("Tab", "Focus: CORES / PROCESSES"),
        ("←↑↓→", "Navigate cores (CORES focus)"),
        ("↑ ↓", "Select process (PROCESSES focus)"),
        ("Enter", "Filter by core"),
        ("m / Esc", "Close menu"),
        ("c", "Full command"),
        ("/", "Search"),
        ("z", "Pause"),
        ("q", "Quit"),
    ];
    let text: Vec<Line> = items
        .iter()
        .map(|pair| {
            let (k, d) = *pair;
            Line::from(vec![
                Span::styled(format!(" {k:<5}"), Style::default().fg(ui.theme.yellow)),
                Span::styled(d, Style::default().fg(ui.theme.fg)),
            ])
        })
        .collect();
    let h = items.len() as u16 + 2;
    let w: u16 = 30;
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let menu_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(" MENU ");
    frame.render_widget(Clear, menu_area);
    frame.render_widget(block.clone(), menu_area);
    frame.render_widget(Paragraph::new(text), block.inner(menu_area));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_bytes_scales() {
        assert_eq!(short_bytes(0), "0.0B");
        assert_eq!(short_bytes(1024), "1.0K");
        assert_eq!(short_bytes(20_000), "20K");
        assert_eq!(short_bytes(535_000_000), "510M");
        assert_eq!(short_bytes(8_900_000_000), "8.3G");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(1024), "1.0KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0MiB");
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn truncate_ellipsizes_long_strings() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("日本語テキスト", 3), "日本…");
        assert_eq!(truncate("exactly-nine", 9), "exactly-…");
    }

    #[test]
    fn bar_fills_and_clamps() {
        assert_eq!(bar(50.0, 10), "+++++·····");
        assert_eq!(bar(200.0, 4), "@@@@");
        assert_eq!(bar(0.0, 4), "····");
    }

    #[test]
    fn bar_glyph_scales_from_light_to_heavy() {
        assert_eq!(bar_glyph(0.0), '.');
        assert_eq!(bar_glyph(50.0), '+');
        assert_eq!(bar_glyph(100.0), '@');
    }

    #[test]
    fn io_header_matches_data_cell_width() {
        assert_eq!(io_header("r/s", 7).chars().count(), 7);
        assert_eq!(io_header("r_awt ms", 8).chars().count(), 8);
        assert_eq!(io_header("busy", 6).chars().count(), 6);
        assert_eq!(format!(" {:>7.1}", 1.0).chars().count(), 8);
    }

    #[test]
    fn ghz_format() {
        assert_eq!(ghz(500), "500M");
        assert_eq!(ghz(1000), "1.0G");
        assert_eq!(ghz(4900), "4.9G");
    }

    fn disk(name: &str) -> crate::data::disk::DiskInfo {
        crate::data::disk::DiskInfo {
            name: name.into(),
            mount: "/".into(),
            fs: "btrfs".into(),
            total_bytes: 100,
            available_bytes: 50,
            used_bytes: 50,
            percent: 50.0,
            read_bps: 0,
            write_bps: 0,
            total_read_bytes: 0,
            total_written_bytes: 0,
            temp_c: None,
            io: Default::default(),
        }
    }

    #[test]
    fn unique_disks_dedupes_by_device() {
        let d = disk("/dev/mapper/root");
        let boot = disk("/dev/nvme0n1p1");
        let disks = [d.clone(), d.clone(), boot, d];
        let uniq: Vec<&String> = unique_disks(&disks).iter().map(|x| &x.name).collect();
        assert_eq!(uniq, vec!["/dev/mapper/root", "/dev/nvme0n1p1"]);
    }

    #[test]
    fn unique_disks_caps_at_five() {
        let disks: Vec<_> = (0..8).map(|i| disk(&format!("/dev/disk{i}"))).collect();
        assert_eq!(unique_disks(&disks).len(), 5);
    }

    #[test]
    fn sparkline_buckets_and_scales() {
        let mut q = VecDeque::new();
        for v in [0.0, 0.0, 100.0, 0.0, 0.0, 0.0] {
            q.push_back(v);
        }
        // 6 samples into 2 buckets: [0,0,100] avg 33.3, [0,0,0] avg 0.
        // Absolute scale to 100: 33.3/100*8=2.66→⡆, 0→⡀.
        let s = sparkline(&q, 2, Some(100.0));
        assert_eq!(s.chars().count(), 2);
        assert_eq!(s, "⡆⡀");
        assert_eq!(sparkline(&VecDeque::new(), 3, Some(100.0)), "   ");
    }

    #[test]
    fn sparkline_auto_scales_rates_without_percent_saturation() {
        let q = VecDeque::from([0.0, 842_000_000.0, 0.0]);
        assert_eq!(sparkline(&q, 3, None), "⡀⣿⡀");
    }

    #[test]
    fn sparkline_handles_zero_width() {
        let samples = VecDeque::from([10.0, 20.0]);
        assert_eq!(sparkline(&samples, 0, None), "");
    }

    #[test]
    fn temp_color_thresholds() {
        let t = Theme::DEFAULT;
        assert_eq!(temp_color(Some(40.0), &t), t.green);
        assert_eq!(temp_color(Some(60.0), &t), t.yellow);
        assert_eq!(temp_color(Some(80.0), &t), t.red);
        assert_eq!(temp_color(None, &t), t.green);
    }
}

//! The I/O pane and its dashboard summary.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::data::cpu::ProcessInfo;

use super::draw_summary;
use super::format::{
    await_color, block, busy_color, io_pressure_color, queue_color, short_bytes, sparkline,
    temp_color, truncate, unique_disks,
};
use super::{Pane, Ui};

/// Full-pane disk I/O view (menu 2 -> IO): PSI pressure, per-disk iostat-style
/// columns, then the top processes actually moving data.
pub(super) fn draw_io(frame: &mut Frame, area: Rect, ui: &Ui) {
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

pub(super) fn io_header(label: &str, width: usize) -> String {
    format!("{label:>width$}")
}

pub(super) fn draw_io_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
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

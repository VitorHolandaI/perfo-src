//! The CPU, memory, disk and process panes.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row as TableRow, Table, TableState},
    Frame,
};

use crate::data::cpu::CoreType;

/// Width the CPU/memory labels occupy before their bar starts.
const OVERALL_LABEL_WIDTH: u16 = 26;
const MEM_LABEL_WIDTH: usize = 8;
const DISK_LABEL_WIDTH: u16 = 30;
/// Width the GPU process row reserves before its command column.
const GPU_ROW_LABEL_WIDTH: usize = 45;
/// How far a mount point is truncated in the disk table.
const MOUNT_WIDTH: usize = 12;
/// Command column width, fullscreen and compact.
const CMD_WIDTH_FULL: usize = 500;
const CMD_WIDTH_COMPACT: usize = 120;
/// Below this width the core grid drops to one column.
const TWO_CORES_PER_LINE_ABOVE: u16 = 60;
/// Height of the block above the process table.
const CPU_HEADER_HEIGHT: u16 = 6;
/// PSI pressure thresholds (percent of the last 10s stalled).
const PSI_HOT: f64 = 10.0;
const PSI_WARN: f64 = 5.0;

use super::format::{
    bar, bar_glyph, block, cpu_color, freq_color, ghz, human_bytes, short_bytes, sparkline,
    truncate, truncate_with_scroll, unique_disks, DISK_HOT_PCT, DISK_WARN_PCT,
};
use super::{Pane, SortKey, Ui};

use super::overlay::draw_trace;
use super::{draw_summary, MAX_CORE_ROWS};

pub(super) fn draw_cpu(frame: &mut Frame, area: Rect, ui: &Ui, framed: bool) {
    let title = match ui.core_filter {
        Some(c) => format!("1:CPU: core {c}"),
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
        Layout::vertical([Constraint::Length(CPU_HEADER_HEIGHT), Constraint::Min(0)]).areas(inner);

    let bar_w = overall_area.width.saturating_sub(OVERALL_LABEL_WIDTH) as usize;
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
    let load = format!("load {:.2} {:.2} {:.2}", la[0], la[1], la[2]);
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
            Line::from(Span::raw(format!("{load}{temp}"))),
            legend,
        ]),
        overall_area,
    );
    draw_cores(frame, cores_area, ui);
}

pub(super) fn draw_cores(frame: &mut Frame, area: Rect, ui: &Ui) {
    let n = ui.snap.per_core.len().min(MAX_CORE_ROWS);
    let two_per_line = area.width >= TWO_CORES_PER_LINE_ABOVE;
    let bar_w = if two_per_line {
        (area.width.saturating_sub(1) / 2).saturating_sub(OVERALL_LABEL_WIDTH) as usize
    } else {
        area.width.saturating_sub(DISK_LABEL_WIDTH) as usize
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

pub(super) fn draw_mem(frame: &mut Frame, area: Rect, ui: &Ui) {
    let focused = ui.pane == Pane::Cpu;
    frame.render_widget(block("4:MEM", focused, &ui.theme), area);
    let inner = block("4:MEM", focused, &ui.theme).inner(area);
    let m = &ui.snap.mem;
    let w = inner.width as usize;
    let bar_w = w.saturating_sub(MEM_LABEL_WIDTH);
    let frac = |x: u64| (x as f64 / m.total.max(1) as f64 * bar_w as f64) as usize;

    let used_w = frac(m.used);
    let cache_w = frac(m.cache);
    let buf_w = frac(m.buffers);
    let free_w = bar_w.saturating_sub(used_w + cache_w + buf_w);
    let pct = crate::units::percent_of(m.used, m.total);

    let bar_line = Line::from(vec![
        Span::styled(
            bar_glyph(pct).to_string().repeat(used_w),
            Style::default().fg(ui.theme.green),
        ),
        Span::styled(
            bar_glyph(crate::units::percent_of(m.cache, m.total))
                .to_string()
                .repeat(cache_w),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            bar_glyph(crate::units::percent_of(m.buffers, m.total))
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
        let swap_pct = crate::units::percent_of(m.swap_used, m.swap_total);
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

    let psi_color = if m.psi_some_10 > PSI_HOT {
        ui.theme.red
    } else if m.psi_some_10 > PSI_WARN {
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

pub(super) fn draw_disks(frame: &mut Frame, area: Rect, ui: &Ui) {
    frame.render_widget(block("5:DISKS", false, &ui.theme), area);
    let inner = block("5:DISKS", false, &ui.theme).inner(area);
    let mut lines: Vec<Line> = Vec::new();
    let w = inner.width as usize;
    // name(12) + 1 + bar + pct(18) + 2 + mount(12)
    let bar_w = w.saturating_sub(GPU_ROW_LABEL_WIDTH);
    for d in unique_disks(&ui.snap.disks) {
        let color = if d.percent >= DISK_HOT_PCT {
            ui.theme.red
        } else if d.percent >= DISK_WARN_PCT {
            ui.theme.yellow
        } else {
            ui.theme.green
        };
        let filled = crate::units::fill_width(d.percent, bar_w as u16) as usize;
        let filled = filled.min(bar_w);
        let name = d.name.rsplit('/').next().unwrap_or(&d.name);
        let mount = truncate(&d.mount, MOUNT_WIDTH);
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

pub(super) fn draw_processes(frame: &mut Frame, area: Rect, ui: &Ui, framed: bool) {
    if ui.tracing {
        draw_trace(frame, area, ui);
        return;
    }
    let title = match (ui.tree, ui.core_filter) {
        (true, Some(c)) => format!("PROCS: core {c} (tree)"),
        (false, Some(c)) => format!("PROCS: core {c}"),
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
    let cmd_hdr = if ui.cmd_scroll > 0 {
        format!("COMMAND [»{}]", ui.cmd_scroll)
    } else {
        "COMMAND".to_string()
    };
    let header = TableRow::new(vec![
        Cell::from("PID"),
        Cell::from("USER"),
        Cell::from(cpu_hdr),
        Cell::from(mem_hdr),
        Cell::from(cmd_hdr),
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
                truncate_with_scroll(&r.process.cmd, ui.cmd_scroll, CMD_WIDTH_FULL)
            } else {
                truncate_with_scroll(&r.process.cmd, ui.cmd_scroll, CMD_WIDTH_COMPACT)
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

pub(super) fn draw_process_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    let lines: Vec<Line> = ui
        .rows
        .iter()
        .take(area.height.saturating_sub(1) as usize)
        .map(|row| {
            Line::from(format!(
                "{:>6} {:>4.1}% {}",
                row.process.pid,
                row.process.cpu_percent,
                truncate_with_scroll(
                    &row.process.cmd,
                    ui.cmd_scroll,
                    area.width.saturating_sub(15) as usize
                )
            ))
        })
        .collect();
    draw_summary(frame, area, "1:PROCS", lines, &ui.theme);
}

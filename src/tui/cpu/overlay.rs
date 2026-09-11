//! Things drawn on top of a pane: the trace view, status line, help and menu.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::format::{block, truncate};
use super::{Pane, Ui};

pub(super) fn draw_trace(frame: &mut Frame, area: Rect, ui: &Ui) {
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

pub(super) fn draw_status(frame: &mut Frame, area: Rect, ui: &Ui) {
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

pub(super) fn draw_help(frame: &mut Frame, area: Rect, ui: &Ui) {
    let (title, text) = crate::tui::help::page(ui.help_page, ui.lang, &ui.theme);
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

pub(super) fn draw_menu(frame: &mut Frame, area: Rect, ui: &Ui) {
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
        ("7", "History analysis (timeline)"),
        ("? / F1", "Help / Ajuda"),
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
                Span::styled(format!(" {k:<7}"), Style::default().fg(ui.theme.yellow)),
                Span::styled(d, Style::default().fg(ui.theme.fg)),
            ])
        })
        .collect();
    let h = items.len() as u16 + 2;
    let w: u16 = 34;
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

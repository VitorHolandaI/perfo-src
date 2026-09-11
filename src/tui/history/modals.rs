//! The saved-sessions list and the recording picker.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row as TableRow, Table},
    Frame,
};

use super::HistoryState;
use crate::tui::cpu::Ui;

pub(super) fn draw_sessions_modal(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };

    let w = 82u16.min(area.width.saturating_sub(4));
    let h = 16u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, modal_area);

    let title = format!(" SAVED SESSIONS ({}) ", state.saved_recordings.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(title);
    frame.render_widget(block.clone(), modal_area);
    let inner = block.inner(modal_area);

    if inner.height < 4 || inner.width < 20 {
        return;
    }

    let [table_area, help_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let header = TableRow::new(vec![
        Span::styled("ID / FILENAME", Style::default().fg(ui.theme.muted)),
        Span::styled("DATE", Style::default().fg(ui.theme.muted)),
        Span::styled("TIME", Style::default().fg(ui.theme.muted)),
        Span::styled("DUR", Style::default().fg(ui.theme.muted)),
        Span::styled("FOCUS", Style::default().fg(ui.theme.muted)),
        Span::styled("SAMPLES", Style::default().fg(ui.theme.muted)),
    ]);

    let rows: Vec<TableRow> = state
        .saved_recordings
        .iter()
        .enumerate()
        .map(|(idx, rec)| {
            let is_selected = idx == state.selected_session_idx;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ui.theme.fg)
            };
            let arrow = if is_selected { "▶ " } else { "  " };
            let id_str = format!("{}{}", arrow, rec.id);
            TableRow::new(vec![
                Span::styled(id_str, style),
                Span::styled(&rec.date, style),
                Span::styled(&rec.time, style),
                Span::styled(&rec.duration, style),
                Span::styled(&rec.metric_focus, style),
                Span::styled(format!("{}", rec.sample_count), style),
            ])
        })
        .collect();

    if rows.is_empty() {
        let empty_msg = Paragraph::new(
            "No saved sessions found in ~/.local/share/perfo/recordings\nPress 'r' in history view to record a session.",
        )
        .style(Style::default().fg(ui.theme.muted));
        frame.render_widget(empty_msg, table_area);
    } else {
        let widths = [
            Constraint::Length(26),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(0),
        ];
        let table = Table::new(rows, widths).header(header);
        frame.render_widget(table, table_area);
    }

    let help_line = Line::from(vec![
        Span::styled("↑/↓/j/k", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Select  "),
        Span::styled("Enter", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Load/Replay  "),
        Span::styled("d", Style::default().fg(ui.theme.red)),
        Span::raw(" Delete  "),
        Span::styled("Esc/s", Style::default().fg(ui.theme.yellow)),
        Span::raw(" Close"),
    ]);
    frame.render_widget(Paragraph::new(help_line), help_area);
}

/// The subsystems a recording can include, in the order the picker shows
/// them: index, hotkey, label, what it costs, and whether it is on.
fn record_modal_items(
    state: &HistoryState,
) -> [(usize, &'static str, &'static str, &'static str, bool); 6] {
    [
        (
            0,
            "1",
            "CPU",
            "Usage, Cores, Threads, CPU Procs",
            state.recording_mask.cpu,
        ),
        (
            1,
            "2",
            "Memory",
            "RAM, Swap, Process Memory",
            state.recording_mask.mem,
        ),
        (
            2,
            "3",
            "Disk I/O",
            "Read/Write Rates, IO Procs",
            state.recording_mask.io,
        ),
        (
            3,
            "4",
            "Network",
            "Bandwidth, Sockets, Net Procs",
            state.recording_mask.net,
        ),
        (
            4,
            "5",
            "GPU",
            "Usage, VRAM, GPU Procs",
            state.recording_mask.gpu,
        ),
        (
            5,
            "6",
            "NPU",
            "Neural Acceleration Engine",
            state.recording_mask.npu,
        ),
    ]
}

/// One checkbox row per subsystem.
fn subsystem_rows(state: &HistoryState, ui: &Ui, lines: &mut Vec<Line<'static>>) {
    let items = record_modal_items(state);
    for (idx, num, name, desc, checked) in items {
        let is_selected = state.record_modal_idx == idx;
        let prefix = if is_selected { "> " } else { "  " };
        let box_str = if checked { "[x] " } else { "[ ] " };
        let box_style = if checked {
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui.theme.muted)
        };
        let label_style = if is_selected {
            Style::default()
                .fg(ui.theme.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ui.theme.fg)
        };
        let desc_style = Style::default().fg(ui.theme.muted);

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                if is_selected {
                    Style::default().fg(ui.theme.accent)
                } else {
                    Style::default()
                },
            ),
            Span::styled(box_str, box_style),
            Span::styled(format!("{}. {:<8}", num, name), label_style),
            Span::styled(format!(" {}", desc), desc_style),
        ]));
    }
}

/// The START / CANCEL buttons at the foot of the picker.
fn record_modal_buttons(state: &HistoryState, ui: &Ui, lines: &mut Vec<Line<'static>>) {
    let btn_start_selected = state.record_modal_idx == 6;
    let btn_cancel_selected = state.record_modal_idx == 7;

    let start_style = if btn_start_selected {
        Style::default()
            .bg(ui.theme.accent)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(ui.theme.accent)
            .add_modifier(Modifier::BOLD)
    };
    let cancel_style = if btn_cancel_selected {
        Style::default()
            .bg(ui.theme.muted)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ui.theme.muted)
    };

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("[ Start Recording (r / Enter) ]", start_style),
        Span::raw("   "),
        Span::styled("[ Cancel (Esc) ]", cancel_style),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  Space/Enter: Toggle  1-6: Direct Toggle  Esc: Cancel",
        Style::default().fg(ui.theme.muted),
    )]));
}

pub(super) fn draw_record_modal(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let bg = match ui.theme.bg {
        Color::Reset => Color::Black,
        c => c,
    };

    let w = 58u16.min(area.width.saturating_sub(4));
    let h = 13u16.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, modal_area);

    let title = " RECORDING SUBSYSTEMS ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.accent))
        .style(Style::default().bg(bg))
        .title(title);
    frame.render_widget(block.clone(), modal_area);
    let inner = block.inner(modal_area);

    if inner.height < 6 || inner.width < 24 {
        return;
    }

    let mut lines = Vec::new();
    subsystem_rows(state, ui, &mut lines);
    record_modal_buttons(state, ui, &mut lines);
    frame.render_widget(Paragraph::new(lines), inner);
}

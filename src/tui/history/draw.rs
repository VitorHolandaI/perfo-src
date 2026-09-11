//! The history page layout and its control bar.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::chart::draw_timeline_chart;
use super::modals::{draw_record_modal, draw_sessions_modal};
use super::tables::{draw_processes_table, draw_stats_box};
use super::HistoryState;
use super::{HistoryMetric, HistorySpan};
use crate::tui::cpu::{Pane, Ui};

pub fn draw_history(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let outer = crate::tui::cpu::block("7:HISTORY ANALYSIS", ui.pane == Pane::History, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);

    if inner.height < 10 || inner.width < 30 {
        return;
    }

    let [controls_area, sparkline_area, stats_area, table_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(inner);

    draw_controls(frame, controls_area, ui, state);
    draw_timeline_chart(frame, sparkline_area, ui, state);
    draw_stats_box(frame, stats_area, ui, state);
    draw_processes_table(frame, table_area, ui, state);

    if state.sessions_modal {
        draw_sessions_modal(frame, area, ui, state);
    } else if state.record_modal {
        draw_record_modal(frame, area, ui, state);
    }
}

/// The `● REC` / `⏸` badge and its subsystem summary.
fn recording_tag(state: &HistoryState, ui: &Ui) -> Span<'static> {
    if state.is_session_recording {
        let cur = state.session_record_buffer.len();
        let tot = state.target_record_seconds;
        let c_m = cur / 60;
        let c_s = cur % 60;
        let t_m = tot / 60;
        let t_s = tot % 60;
        let summary = state.recording_mask.summary();
        let tag_text = if summary == "ALL" {
            format!("● REC {:02}:{:02}/{:02}:{:02}", c_m, c_s, t_m, t_s)
        } else {
            format!(
                "● REC {:02}:{:02}/{:02}:{:02} [{}]",
                c_m, c_s, t_m, t_s, summary
            )
        };
        Span::styled(
            tag_text,
            Style::default()
                .fg(ui.theme.red)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(title) = &state.loaded_session_title {
        Span::styled(
            format!("▶ REPLAY: {}", title),
            Style::default()
                .fg(ui.theme.yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if state.recording {
        Span::styled(
            "● REC",
            Style::default()
                .fg(ui.theme.red)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("⏸ PAUSED", Style::default().fg(ui.theme.muted))
    }
}

/// `[PLAYING]` while replaying, nothing while live.
fn playback_tag(state: &HistoryState, ui: &Ui) -> Span<'static> {
    if state.playing {
        Span::styled(
            " [PLAYING]",
            Style::default()
                .fg(ui.theme.yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    }
}

/// Where the cursor sits: the timestamp or `LIVE`, and how far into the span.
fn position_tags(state: &HistoryState, ui: &Ui) -> (Span<'static>, usize, usize) {
    let eff = state.effective_index();
    let is_live = state.is_live();
    let total = state.sample_count();
    let span_secs = state.span.seconds(total);
    let start_idx = total.saturating_sub(span_secs);
    let total_span = span_secs.min(total).max(1);
    let elapsed = if is_live {
        total_span
    } else {
        let span_samples = total.saturating_sub(1).saturating_sub(start_idx).max(1);
        let ratio = (eff.saturating_sub(start_idx) as f64) / (span_samples as f64);
        ((ratio * total_span as f64).round() as usize).min(total_span)
    };
    let time_str = state
        .get_sample(eff)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let time_mode = if is_live {
        Span::styled(
            format!(" LIVE [+{}s] ({})", total_span, time_str),
            Style::default()
                .fg(ui.theme.green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" +{}s ({})", elapsed, time_str),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    };

    (time_mode, elapsed, total_span)
}

fn draw_controls(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let rec_tag = recording_tag(state, ui);
    let play_tag = playback_tag(state, ui);
    let (time_mode, elapsed, total_span) = position_tags(state, ui);
    let metric_pill = |m: HistoryMetric| {
        let label = m.label();
        if state.metric == m {
            Span::styled(
                format!(" [{label}] "),
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("  {label}  "), Style::default().fg(ui.theme.fg))
        }
    };

    let span_pill = |sp: HistorySpan| {
        let label = sp.label();
        if state.span == sp {
            Span::styled(
                format!(" [{label}] "),
                Style::default()
                    .fg(Color::Black)
                    .bg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("  {label}  "), Style::default().fg(ui.theme.muted))
        }
    };

    let (el_m, el_s) = crate::units::minutes_seconds(elapsed as u64);
    let (tot_m, tot_s) = crate::units::minutes_seconds(total_span as u64);

    let timer_tag = Span::styled(
        format!(" ⏱ {:02}:{:02}/{:02}:{:02} ", el_m, el_s, tot_m, tot_s),
        Style::default()
            .fg(Color::Black)
            .bg(ui.theme.accent)
            .add_modifier(Modifier::BOLD),
    );

    let line1 = Line::from(vec![
        Span::styled("STATUS: ", Style::default().fg(ui.theme.muted)),
        rec_tag,
        play_tag,
        time_mode,
        Span::raw(" "),
        timer_tag,
        Span::raw("  │  "),
        Span::styled("METRIC: ", Style::default().fg(ui.theme.muted)),
        metric_pill(HistoryMetric::Cpu),
        metric_pill(HistoryMetric::Mem),
        metric_pill(HistoryMetric::Io),
        metric_pill(HistoryMetric::Net),
        metric_pill(HistoryMetric::Gpu),
        Span::raw("  │  "),
        Span::styled("SPAN: ", Style::default().fg(ui.theme.muted)),
        span_pill(HistorySpan::Span2m),
        span_pill(HistorySpan::Span15m),
        span_pill(HistorySpan::Span1h),
        span_pill(HistorySpan::SpanAll),
    ]);

    let export_text = if let Some((msg, inst)) = &state.export_status {
        if inst.elapsed().as_secs() < 4 {
            Span::styled(
                format!("  [{msg}]"),
                Style::default()
                    .fg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        }
    } else {
        Span::raw("")
    };

    let line2 = Line::from(vec![
        Span::styled("KEYS: ", Style::default().fg(ui.theme.muted)),
        Span::styled("< > / ← →", Style::default().fg(ui.theme.yellow)),
        Span::raw(" step  "),
        Span::styled("[ ]", Style::default().fg(ui.theme.yellow)),
        Span::raw(" jump  "),
        Span::styled("Space", Style::default().fg(ui.theme.yellow)),
        Span::raw(" play  "),
        Span::styled("0/L", Style::default().fg(ui.theme.yellow)),
        Span::raw(" live  "),
        Span::styled("Tab", Style::default().fg(ui.theme.yellow)),
        Span::raw(" metric  "),
        Span::styled("z", Style::default().fg(ui.theme.yellow)),
        Span::raw(" span  "),
        Span::styled("r", Style::default().fg(ui.theme.yellow)),
        Span::raw(" rec  "),
        Span::styled("s", Style::default().fg(ui.theme.yellow)),
        Span::raw(" sessions  "),
        Span::styled("e", Style::default().fg(ui.theme.yellow)),
        Span::raw(" export"),
        export_text,
    ]);

    let par = Paragraph::new(vec![line1, line2]);
    frame.render_widget(par, area);
}

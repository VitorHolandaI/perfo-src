//! The timeline chart: the densest drawing in the TUI.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::{HistoryMetric, HistoryState};
use crate::tui::cpu::Ui;

/// Rate metrics never scale below this, so an idle chart still has a ruler.
const MIN_IO_SCALE: f32 = 10.0;
const MIN_NET_SCALE: f32 = 100_000.0;
/// Block glyphs run from empty to full; this is the index of the last one.
const TOP_GLYPH: usize = 7;
/// Peaks at or above these paint the bar in the warning colours.
const PEAK_HOT_PCT: f32 = 80.0;
const PEAK_WARN_PCT: f32 = 50.0;
/// The ruler marks quarters of the visible span.
const RULER_DIVISIONS: usize = 4;

/// Where the chart's columns land: how many there are, which samples they
/// cover, and which one carries the cursor.
struct ChartGeometry {
    w: usize,
    start_idx: usize,
    total: usize,
    step: f64,
    cursor_pos: usize,
}

/// The value the tallest bar represents. Percentages are fixed at 100; the
/// rate metrics scale to the busiest sample in view.
fn metric_ceiling(state: &HistoryState, total: usize) -> f32 {
    match state.metric {
        HistoryMetric::Cpu | HistoryMetric::Mem | HistoryMetric::Gpu => 100.0f32,
        HistoryMetric::Io => {
            let mut m = MIN_IO_SCALE;
            for idx in 0..total {
                if let Some(s) = state.get_sample(idx) {
                    if s.io_mb > m {
                        m = s.io_mb;
                    }
                }
            }
            m
        }
        HistoryMetric::Net => {
            let mut m = MIN_NET_SCALE;
            for idx in 0..total {
                if let Some(s) = state.get_sample(idx) {
                    let v = (s.net_rx_bps + s.net_tx_bps) as f32;
                    if v > m {
                        m = v;
                    }
                }
            }
            m
        }
    }
}

/// One block glyph per terminal column, each column covering `step` samples
/// and showing that window's peak.
fn bar_column_spans(
    state: &HistoryState,
    ui: &Ui,
    geom: &ChartGeometry,
    max_metric_val: f32,
) -> Vec<Span<'static>> {
    let ChartGeometry {
        w,
        start_idx,
        total,
        step,
        cursor_pos,
    } = *geom;
    let mut bar_spans = Vec::with_capacity(w);
    const GLYPHS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    for col in 0..w {
        let b_start = (start_idx as f64 + col as f64 * step).floor() as usize;
        let b_end = (start_idx as f64 + (col + 1) as f64 * step).ceil() as usize;
        let b_end = b_end.min(total).max(b_start + 1);

        let mut peak = 0.0f32;
        for idx in b_start..b_end.min(total) {
            if let Some(s) = state.get_sample(idx) {
                let v = match state.metric {
                    HistoryMetric::Cpu => s.cpu,
                    HistoryMetric::Mem => s.mem,
                    HistoryMetric::Io => s.io_mb,
                    HistoryMetric::Net => (s.net_rx_bps + s.net_tx_bps) as f32,
                    HistoryMetric::Gpu => s.gpu,
                };
                if v > peak {
                    peak = v;
                }
            }
        }

        let ratio = (peak / max_metric_val).clamp(0.0, 1.0);
        let g_idx = ((ratio * TOP_GLYPH as f32).round() as usize).min(TOP_GLYPH);
        let ch = GLYPHS[g_idx];

        let is_cur = col == cursor_pos;
        let style = if is_cur {
            Style::default()
                .fg(Color::Black)
                .bg(ui.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if peak >= PEAK_HOT_PCT
            && state.metric != HistoryMetric::Io
            && state.metric != HistoryMetric::Net
        {
            Style::default().fg(ui.theme.red)
        } else if peak >= PEAK_WARN_PCT
            && state.metric != HistoryMetric::Io
            && state.metric != HistoryMetric::Net
        {
            Style::default().fg(ui.theme.yellow)
        } else {
            Style::default().fg(ui.theme.accent)
        };

        bar_spans.push(Span::styled(ch.to_string(), style));
    }
    bar_spans
}

/// Tick marks under the bars, one per column.
fn ruler_column_spans(ui: &Ui, w: usize, cursor_pos: usize) -> Vec<Span<'static>> {
    let mut ruler_spans: Vec<Span<'static>> = Vec::with_capacity(w);
    for col in 0..w {
        if col == cursor_pos {
            ruler_spans.push(Span::styled(
                "▲",
                Style::default()
                    .fg(ui.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        } else if col == 0 || col == w.saturating_sub(1) {
            ruler_spans.push(Span::styled("|", Style::default().fg(ui.theme.fg)));
        } else if col == w / RULER_DIVISIONS || col == w / 2 || col == (3 * w) / 4 {
            ruler_spans.push(Span::styled("+", Style::default().fg(ui.theme.muted)));
        } else {
            ruler_spans.push(Span::styled("-", Style::default().fg(ui.theme.muted)));
        }
    }
    ruler_spans
}

/// The two axis lines under the chart: wall-clock timestamps, and elapsed
/// time against the span.
fn axis_lines(
    state: &HistoryState,
    ui: &Ui,
    w: usize,
    start_idx: usize,
    span_secs: usize,
    total: usize,
) -> (Line<'static>, Line<'static>) {
    let eff = state.effective_index();
    let start_time = state
        .get_sample(start_idx)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let end_time = state
        .last_sample()
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");
    let cur_time = state
        .get_sample(eff)
        .map(|s| s.timestamp.as_str())
        .unwrap_or("--");

    let total_span = span_secs.min(total).max(1);
    let elapsed = if state.is_live() {
        total_span
    } else {
        let span_samples = total.saturating_sub(1).saturating_sub(start_idx).max(1);
        let ratio = (eff.saturating_sub(start_idx) as f64) / (span_samples as f64);
        ((ratio * total_span as f64).round() as usize).min(total_span)
    };
    let (el_m, el_s) = crate::units::minutes_seconds(elapsed as u64);
    let (tot_m, tot_s) = crate::units::minutes_seconds(total_span as u64);

    let cur_str = if state.is_live() {
        format!("+{}s LIVE", elapsed)
    } else {
        format!("+{}s", elapsed)
    };

    let time_axis_line = Line::from(vec![
        Span::styled(
            format!("{:<12}", start_time),
            Style::default().fg(ui.theme.muted),
        ),
        Span::styled(
            format!(
                "{:^width$}",
                format!("▲ {} ({})", cur_time, cur_str),
                width = w.saturating_sub(24)
            ),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:>12}", end_time),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    let timer_axis_line = Line::from(vec![
        Span::styled(
            format!("{:<12}", "+0s"),
            Style::default().fg(ui.theme.muted),
        ),
        Span::styled(
            format!(
                "{:^width$}",
                format!("[⏱ {:02}:{:02} / {:02}:{:02}]", el_m, el_s, tot_m, tot_s),
                width = w.saturating_sub(24)
            ),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("{:>12}", format!("+{}s LIVE", total_span)),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    (time_axis_line, timer_axis_line)
}

pub(super) fn draw_timeline_chart(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.muted))
        .title(format!(" TIMELINE GRAPH ({}) ", state.metric.label()));
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let total = state.sample_count();
    let span_secs = state.span.seconds(total);
    let start_idx = total.saturating_sub(span_secs);
    let visible_count = total.saturating_sub(start_idx);

    if visible_count == 0 {
        let msg = Paragraph::new("Collecting history samples...")
            .style(Style::default().fg(ui.theme.muted));
        frame.render_widget(msg, inner);
        return;
    }

    let w = inner.width as usize;
    let eff = state.effective_index();

    let mut cursor_chars = vec![' '; w];

    let step = (visible_count as f64) / (w as f64);
    let cursor_pos = if eff >= start_idx && visible_count > 0 {
        let rel = eff - start_idx;
        ((rel as f64 / visible_count as f64) * (w as f64 - 1.0)).round() as usize
    } else {
        w.saturating_sub(1)
    };

    if cursor_pos < w {
        cursor_chars[cursor_pos] = '▼';
    }
    let max_metric_val = metric_ceiling(state, total);
    let geom = ChartGeometry {
        w,
        start_idx,
        total,
        step,
        cursor_pos,
    };
    let bar_spans = bar_column_spans(state, ui, &geom, max_metric_val);
    let ruler_spans = ruler_column_spans(ui, w, cursor_pos);

    let cursor_line = Line::from(Span::styled(
        cursor_chars.into_iter().collect::<String>(),
        Style::default()
            .fg(ui.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    let bars_line = Line::from(bar_spans);

    let (time_axis_line, timer_axis_line) = axis_lines(state, ui, w, start_idx, span_secs, total);
    let ruler_line = Line::from(ruler_spans);

    let par = Paragraph::new(vec![
        cursor_line,
        bars_line,
        ruler_line,
        time_axis_line,
        timer_axis_line,
    ]);
    frame.render_widget(par, inner);
}

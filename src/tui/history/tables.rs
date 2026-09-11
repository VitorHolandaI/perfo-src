//! The statistics box and the per-process table under the chart.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row as TableRow, Table},
    Frame,
};

use super::format::clean_process_name;
use super::{HistoryMetric, HistoryProcess, HistoryState};
use crate::tui::cpu::Ui;

pub(super) fn draw_stats_box(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let eff = state.effective_index();
    let sample = state.get_sample(eff);

    let (cur_cpu, cur_mem, cur_io, cur_net, cur_gpu) = match sample {
        Some(s) => (s.cpu, s.mem, s.io_mb, s.net_rx_bps + s.net_tx_bps, s.gpu),
        None => (0.0, 0.0, 0.0, 0, 0.0),
    };

    let mut peak_cpu = 0.0f32;
    let mut peak_mem = 0.0f32;
    let mut peak_io = 0.0f32;
    let mut peak_net = 0u64;
    let mut peak_gpu = 0.0f32;
    let mut total_cpu = 0.0f64;
    let mut total_mem = 0.0f64;

    let count = state.sample_count();
    for idx in 0..count {
        if let Some(s) = state.get_sample(idx) {
            total_cpu += s.cpu as f64;
            total_mem += s.mem as f64;
            if s.cpu > peak_cpu {
                peak_cpu = s.cpu;
            }
            if s.mem > peak_mem {
                peak_mem = s.mem;
            }
            if s.io_mb > peak_io {
                peak_io = s.io_mb;
            }
            let net_tot = s.net_rx_bps + s.net_tx_bps;
            if net_tot > peak_net {
                peak_net = net_tot;
            }
            if s.gpu > peak_gpu {
                peak_gpu = s.gpu;
            }
        }
    }

    let cnt_f64 = count.max(1) as f64;
    let avg_cpu = total_cpu / cnt_f64;
    let avg_mem = total_mem / cnt_f64;

    let line1 = Line::from(vec![
        Span::styled("SAMPLE: ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:>4.1}%  ", cur_cpu),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("MEM {:>4.1}%  ", cur_mem),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("IO {:>5.1} MB/s  ", cur_io),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("NET {:>8}/s  ", crate::tui::cpu::human_bytes(cur_net)),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("GPU {:>4.1}%", cur_gpu),
            Style::default()
                .fg(ui.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::styled("PEAKS : ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:.1}%  ", peak_cpu),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("MEM {:.1}%  ", peak_mem),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("IO {:.1} MB/s  ", peak_io),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("NET {}/s  ", crate::tui::cpu::human_bytes(peak_net)),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(
            format!("GPU {:.1}%  │  ", peak_gpu),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled("AVG: ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("CPU {:.1}%  ", avg_cpu),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("MEM {:.1}%  ", avg_mem),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("({} samples)", count),
            Style::default().fg(ui.theme.muted),
        ),
    ]);

    let par = Paragraph::new(vec![line1, line2]);
    frame.render_widget(par, area);
}

/// The NET metric gets its own table: RX, TX and connection counts instead of
/// the generic primary/secondary columns.
fn draw_net_processes_table(
    frame: &mut Frame,
    ui: &Ui,
    procs: &[HistoryProcess],
    inner: Rect,
    max_rows: usize,
) {
    let header = TableRow::new(vec![
        Span::styled("PID", Style::default().fg(ui.theme.muted)),
        Span::styled("PROCESS", Style::default().fg(ui.theme.muted)),
        Span::styled("IN (RX)", Style::default().fg(ui.theme.muted)),
        Span::styled("OUT (TX)", Style::default().fg(ui.theme.muted)),
        Span::styled("TOTAL / CONNS", Style::default().fg(ui.theme.muted)),
        Span::styled("COMMAND", Style::default().fg(ui.theme.muted)),
    ]);

    let rows: Vec<TableRow> = procs
        .iter()
        .take(max_rows)
        .map(|p| {
            let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
            let rx_str = format_proc_net_io(p.net_rx_bps, p.net_rx_bytes);
            let tx_str = format_proc_net_io(p.net_tx_bps, p.net_tx_bytes);
            let conns_str = format_proc_conns(p);
            TableRow::new(vec![
                Span::styled(format!("{:<7}", p.pid), Style::default().fg(ui.theme.fg)),
                Span::styled(
                    p_name,
                    Style::default()
                        .fg(ui.theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(rx_str, Style::default().fg(ui.theme.accent)),
                Span::styled(tx_str, Style::default().fg(ui.theme.accent)),
                Span::styled(conns_str, Style::default().fg(ui.theme.fg)),
                Span::styled(p.cmd.clone(), Style::default().fg(ui.theme.muted)),
            ])
        })
        .collect();

    if rows.is_empty() {
        let p = Paragraph::new("No network socket activity recorded for this sample")
            .style(Style::default().fg(ui.theme.muted));
        frame.render_widget(p, inner);
        return;
    }

    let widths = [
        Constraint::Length(8),
        Constraint::Length(16),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(22),
        Constraint::Min(0),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, inner);
}

pub(super) fn draw_processes_table(frame: &mut Frame, area: Rect, ui: &Ui, state: &HistoryState) {
    let eff = state.effective_index();
    let sample = state.get_sample(eff);

    let mut procs = sample.map(|s| s.top_procs.clone()).unwrap_or_default();
    state.sort_processes(&mut procs);

    let title = format!(
        " ACTIVE PROCESSES AT SELECTED TIMING ({}) ",
        sample.map(|s| s.timestamp.as_str()).unwrap_or("--")
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ui.theme.muted))
        .title(title);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);

    let max_rows = inner.height.saturating_sub(1) as usize;

    if state.metric == HistoryMetric::Net {
        draw_net_processes_table(frame, ui, &procs, inner, max_rows);
        return;
    }

    let primary_hdr = match state.metric {
        HistoryMetric::Cpu => "CPU%",
        HistoryMetric::Mem => "MEM%",
        HistoryMetric::Io => "IO RATE",
        HistoryMetric::Gpu => "GPU%",
        HistoryMetric::Net => "NET",
    };
    let secondary_hdr = match state.metric {
        HistoryMetric::Gpu => "VRAM",
        _ => "RAM",
    };

    let header = TableRow::new(vec![
        Span::styled("PID", Style::default().fg(ui.theme.muted)),
        Span::styled("PROCESS", Style::default().fg(ui.theme.muted)),
        Span::styled(primary_hdr, Style::default().fg(ui.theme.muted)),
        Span::styled(secondary_hdr, Style::default().fg(ui.theme.muted)),
        Span::styled("COMMAND", Style::default().fg(ui.theme.muted)),
    ]);

    let rows: Vec<TableRow> = procs
        .iter()
        .take(max_rows)
        .map(|p| {
            let p_name = clean_process_name(&p.name, &p.cmd, p.pid);
            let primary = state.metric_cell_text(p);
            let secondary = if state.metric == HistoryMetric::Gpu {
                crate::tui::cpu::human_bytes(p.vram_bytes)
            } else {
                crate::tui::cpu::human_bytes(p.mem_bytes)
            };
            TableRow::new(vec![
                Span::styled(format!("{:<7}", p.pid), Style::default().fg(ui.theme.fg)),
                Span::styled(
                    p_name,
                    Style::default()
                        .fg(ui.theme.fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(primary, Style::default().fg(ui.theme.accent)),
                Span::styled(secondary, Style::default().fg(ui.theme.fg)),
                Span::styled(p.cmd.clone(), Style::default().fg(ui.theme.muted)),
            ])
        })
        .collect();

    if rows.is_empty() {
        let empty_msg = if state.metric == HistoryMetric::Gpu {
            "No active GPU processes recorded for this sample"
        } else {
            "No process activity recorded for this sample"
        };
        let p = Paragraph::new(empty_msg).style(Style::default().fg(ui.theme.muted));
        frame.render_widget(p, inner);
        return;
    }

    let widths = [
        Constraint::Length(8),
        Constraint::Length(18),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Min(0),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, inner);
}

pub(super) fn format_proc_net_io(bps: u64, bytes: u64) -> String {
    if bps > 0 {
        format!("{}/s", crate::tui::cpu::human_bytes(bps))
    } else if bytes > 0 {
        crate::tui::cpu::human_bytes(bytes)
    } else {
        "--".to_string()
    }
}

pub(super) fn format_proc_conns(proc: &HistoryProcess) -> String {
    let tot_bytes = proc.net_rx_bytes + proc.net_tx_bytes;
    let tot_str = if tot_bytes > 0 {
        format!("Tot {}", crate::tui::cpu::human_bytes(tot_bytes))
    } else {
        String::new()
    };
    let mut conns = Vec::new();
    if proc.tcp_est > 0 {
        conns.push(format!("{} est", proc.tcp_est));
    }
    if proc.udp > 0 {
        conns.push(format!("{} udp", proc.udp));
    }
    let conn_str = conns.join(", ");
    if !tot_str.is_empty() && !conn_str.is_empty() {
        format!("{} │ {}", tot_str, conn_str)
    } else if !tot_str.is_empty() {
        tot_str
    } else if !conn_str.is_empty() {
        conn_str
    } else {
        "--".to_string()
    }
}

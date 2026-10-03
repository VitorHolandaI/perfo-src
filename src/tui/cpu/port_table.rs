//! The PORT TRAFFIC table of the network pane.
//!
//! Split out of `net.rs`, which owns the interface rows and the pane frame.
//! `draw_net` there is the only caller.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use std::collections::VecDeque;

use crate::data::net::PortSide;

use super::format::{human_bytes, pipe, short_bytes, sparkline_recent};
use super::Ui;

/// How many port rows fit the pane without pushing the listening table off.
const MAX_PORT_ROWS: usize = 12;

/// How many samples of each rate ring the row draws.
const PORT_GRAPH_WIDTH: usize = 20;

/// The rate one port row is drawn against: its own peak inside the drawn
/// window, which is what makes the line use the full height and move.
///
/// Deliberately per row, the way bpytop scales each graph. Two rows are not
/// comparable by height, and that is fine: the rate column sits right beside
/// the graph and carries the magnitude. One scale shared across the table was
/// tried and makes every row quieter than the busiest port collapse to a flat
/// line, which throws away the history the graph exists to show.
///
/// `None` when the window is all zeros, so an idle row draws its baseline
/// instead of dividing by zero.
fn row_scale(hist: &VecDeque<f32>, width: usize) -> Option<f32> {
    let peak = hist
        .iter()
        .rev()
        .take(width)
        .copied()
        .fold(0.0f32, f32::max);
    (peak > 0.0).then_some(peak)
}

/// Which ports are moving bytes, listening or not. Fullscreen only.
///
/// TCP only, and labelled as such: the kernel keeps no cumulative byte
/// counter for UDP sockets, so QUIC, DNS and WireGuard never show up here.
pub(super) fn port_traffic_rows(ui: &Ui, focused: bool, lines: &mut Vec<Line<'static>>) {
    let busy: Vec<_> = ui
        .snap
        .net
        .ports
        .iter()
        .filter(|p| p.rx_bytes + p.tx_bytes > 0)
        .take(MAX_PORT_ROWS)
        .collect();
    if !focused || busy.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "PORT TRAFFIC (TCP only)",
        Style::default().fg(ui.theme.accent),
    )));
    lines.push(Line::from(vec![
        pipe(&format!("  {:>5} {:<4}", "PORT", "DIR"), &ui.theme),
        pipe(
            &format!("{:<w$} {:>10} ", "", "RX/s", w = PORT_GRAPH_WIDTH),
            &ui.theme,
        ),
        pipe(
            &format!("{:<w$} {:>10} ", "", "TX/s", w = PORT_GRAPH_WIDTH),
            &ui.theme,
        ),
        pipe(&format!("{:>12}", "TOTAL RX"), &ui.theme),
        pipe(&format!("{:>12}", "TOTAL TX"), &ui.theme),
        pipe(&format!("{:>7}", "CONNS"), &ui.theme),
    ]));
    for p in busy {
        let (dir, dir_color) = match p.side {
            PortSide::Local => ("in", ui.theme.green),
            PortSide::Remote => ("out", ui.theme.muted),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:>5}", p.port),
                Style::default().fg(ui.theme.yellow),
            ),
            Span::styled(format!(" {dir:<4}"), Style::default().fg(dir_color)),
            Span::styled(
                format!(
                    "{:<w$} {:>10} ",
                    sparkline_recent(
                        &p.rx_hist,
                        PORT_GRAPH_WIDTH,
                        row_scale(&p.rx_hist, PORT_GRAPH_WIDTH),
                    ),
                    format!("{}/s", short_bytes(p.rx_bps)),
                    w = PORT_GRAPH_WIDTH
                ),
                Style::default().fg(ui.theme.accent),
            ),
            Span::styled(
                format!(
                    "{:<w$} {:>10} ",
                    sparkline_recent(
                        &p.tx_hist,
                        PORT_GRAPH_WIDTH,
                        row_scale(&p.tx_hist, PORT_GRAPH_WIDTH),
                    ),
                    format!("{}/s", short_bytes(p.tx_bps)),
                    w = PORT_GRAPH_WIDTH
                ),
                Style::default().fg(ui.theme.yellow),
            ),
            Span::styled(
                format!("{:>12}", human_bytes(p.rx_bytes)),
                Style::default().fg(ui.theme.fg),
            ),
            Span::styled(
                format!("{:>12}", human_bytes(p.tx_bytes)),
                Style::default().fg(ui.theme.fg),
            ),
            Span::styled(
                format!("{:>7}", p.connections),
                Style::default().fg(ui.theme.muted),
            ),
        ]));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{port_traffic_rows, PortSide, Ui};
    use crate::data::cpu::CpuSnapshot;
    use crate::data::disk::HISTORY_SAMPLES;
    use crate::data::net::PortTraffic;
    use crate::theme::Theme;
    use crate::tui::cpu::{Pane, Row, SortKey};

    /// A port row whose rate ring follows `shape` over the full 120 samples.
    fn busy_port(port: u16, side: PortSide, shape: impl Fn(usize) -> (f32, f32)) -> PortTraffic {
        let mut rx_hist = VecDeque::new();
        let mut tx_hist = VecDeque::new();
        let (mut rx_total, mut tx_total) = (0u64, 0u64);
        for i in 0..HISTORY_SAMPLES {
            let (rx, tx) = shape(i);
            rx_hist.push_back(rx);
            tx_hist.push_back(tx);
            rx_total += rx as u64;
            tx_total += tx as u64;
        }
        PortTraffic {
            port,
            side,
            rx_bytes: rx_total,
            tx_bytes: tx_total,
            rx_bps: *rx_hist.back().unwrap() as u64,
            tx_bps: *tx_hist.back().unwrap() as u64,
            connections: 1 + (port % 7) as u32,
            rx_hist,
            tx_hist,
        }
    }

    /// Saturated link rates, so the table is checked where the numbers are
    /// widest: the cells are fixed, and `12 GB/s` must not shove a column.
    fn saturated_ports() -> Vec<PortTraffic> {
        let gb = 1_000_000_000.0f32;
        vec![
            // Sustained 40 Gb/s download, mild jitter.
            busy_port(443, PortSide::Remote, |i| {
                (4.8 * 1e9 + (i as f32 * 0.7).sin() * 2e8, 1.1e8)
            }),
            // Ramp: idle to saturated across the window.
            busy_port(6443, PortSide::Remote, |i| {
                (i as f32 / 120.0 * 12.0 * gb, i as f32 / 120.0 * 2e8)
            }),
            // Bursts every 13 samples, idle between.
            busy_port(3306, PortSide::Local, |i| {
                if i % 13 == 0 {
                    (9.4 * gb, 3.2 * gb)
                } else {
                    (2e6, 1e6)
                }
            }),
            // Sawtooth.
            busy_port(8080, PortSide::Local, |i| {
                ((i % 20) as f32 / 20.0 * 6.0 * gb, (i % 20) as f32 * 1e7)
            }),
            // One huge spike long ago, idle since: the old ring kept smearing
            // it across a whole column forever.
            busy_port(445, PortSide::Local, |i| {
                if i == 12 {
                    (18.0 * gb, 18.0 * gb)
                } else {
                    (0.0, 0.0)
                }
            }),
            // Steady upload, nothing coming back.
            busy_port(22, PortSide::Local, |_| (4.1e6, 2.7 * gb)),
        ]
    }

    fn preview_ui<'a>(snap: &'a CpuSnapshot, rows: &'a [Row<'a>]) -> Ui<'a> {
        Ui {
            snap,
            rows,
            selected: None,
            core_focus: 0,
            core_filter: None,
            sort: SortKey::Cpu,
            invert: false,
            full_cmd: false,
            tree: false,
            pane: Pane::Net,
            fullscreen: true,
            theme: Theme::DEFAULT,
            help: false,
            help_page: 0,
            show_menu: false,
            cores_focused: false,
            show_threads: false,
            lang: crate::tui::Lang::En,
            tracing: false,
            trace_lines: None,
            trace_pid: None,
            history: None,
            status: "",
            searching: false,
            kill_prompt: false,
            cmd_scroll: 0,
        }
    }

    /// Saturated rates must not widen any cell: the header and every row stay
    /// the same width, which is what keeps the `│` separators aligned.
    #[test]
    fn port_traffic_rows_hold_their_columns_at_saturated_rates() {
        let mut snap = CpuSnapshot::default();
        snap.net.ports = saturated_ports();
        let ui = preview_ui(&snap, &[]);
        let mut lines = Vec::new();
        port_traffic_rows(&ui, true, &mut lines);

        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        for line in &text {
            println!("{line}");
        }

        // "", title, header, then one row per port.
        assert_eq!(text.len(), 3 + 6);
        let widths: Vec<usize> = text[2..].iter().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "saturated rates broke the column widths: {widths:?}"
        );
    }

    /// Per-port rate generator for the fixture: refresh index -> (rx, tx).
    type RateShape = Box<dyn Fn(usize) -> (f32, f32)>;

    /// Every row must redraw on every refresh, busy or quiet.
    ///
    /// Regression for the table reading as frozen: the graph bucketed the
    /// whole 120-sample ring, and later one scale shared across the table
    /// collapsed every row quieter than the busiest port to a flat line.
    #[test]
    fn port_traffic_rows_redraw_on_every_refresh() {
        let gb = 1_000_000_000.0f32;
        let shapes: Vec<(u16, PortSide, RateShape)> = vec![
            (
                443,
                PortSide::Remote,
                Box::new(move |i: usize| (2.0 * gb + (i as f32 * 0.5).sin() * 0.9 * gb, 4.0e6)),
            ),
            (
                22,
                PortSide::Local,
                Box::new(|i: usize| (4.1e6 + (i as f32 * 0.9).sin() * 1.5e6, 9.0e6)),
            ),
            (
                631,
                PortSide::Local,
                Box::new(|i: usize| (1.2e3 + (i % 5) as f32 * 300.0, 900.0)),
            ),
        ];
        let mut frames: Vec<Vec<String>> = Vec::new();
        for frame in 0..6 {
            let ports: Vec<PortTraffic> = shapes
                .iter()
                .map(|(port, side, shape)| {
                    let mut p = PortTraffic {
                        port: *port,
                        side: *side,
                        connections: 2,
                        ..PortTraffic::default()
                    };
                    for i in 0..(HISTORY_SAMPLES + frame) {
                        let (rx, tx) = shape(i);
                        p.rx_hist.push_back(rx);
                        p.tx_hist.push_back(tx);
                        if p.rx_hist.len() > HISTORY_SAMPLES {
                            p.rx_hist.pop_front();
                            p.tx_hist.pop_front();
                        }
                        p.rx_bytes += rx as u64;
                        p.tx_bytes += tx as u64;
                    }
                    p.rx_bps = *p.rx_hist.back().unwrap() as u64;
                    p.tx_bps = *p.tx_hist.back().unwrap() as u64;
                    p
                })
                .collect();
            let mut snap = CpuSnapshot::default();
            snap.net.ports = ports;
            let ui = preview_ui(&snap, &[]);
            let mut lines = Vec::new();
            port_traffic_rows(&ui, true, &mut lines);
            println!("--- refresh +{frame}s ---");
            let drawn: Vec<String> = lines
                .iter()
                .skip(3)
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            for row in &drawn {
                println!("{}", row.chars().take(70).collect::<String>());
            }
            frames.push(drawn);
        }

        for (n, pair) in frames.windows(2).enumerate() {
            for (row, (before, after)) in pair[0].iter().zip(&pair[1]).enumerate() {
                assert_ne!(
                    before,
                    after,
                    "row {row} did not move between refresh {n} and {}",
                    n + 1
                );
            }
        }
    }
}

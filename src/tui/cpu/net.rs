//! The network pane and its dashboard summary.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::data::net::PortSide;

use super::format::{block, human_bytes, pipe, short_bytes, sparkline, truncate};
use super::{Pane, Ui};

/// How many port rows fit the pane without pushing the listening table off.
const MAX_PORT_ROWS: usize = 12;

/// Full-pane network view (menu 3 -> NET): per-interface rx/tx rates and
/// packet/error/drop counters plus TCP retransmissions and connections.
///
/// Column cells (fixed): IFACE(10) | RX/s(9) | TX/s(9) | PPS(17) |
/// ERR/DROP(13) | LINK(14). The legend line uses the SAME cell widths so
/// every `│` lines up across the three rows.
/// One row per interface: link state, speed and current rates.
fn interface_rows(ui: &Ui, lines: &mut Vec<Line<'static>>) {
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
            pipe("│", &ui.theme),
            Span::styled(
                format!(
                    "{:<10} {:>9}",
                    sparkline(&i.rx_hist, 10, None),
                    format!("{}/s", short_bytes(i.rx_bps))
                ),
                Style::default().fg(ui.theme.accent),
            ),
            pipe("│", &ui.theme),
            Span::styled(
                format!(
                    "{:<10} {:>9}",
                    sparkline(&i.tx_hist, 10, None),
                    format!("{}/s", short_bytes(i.tx_bps))
                ),
                Style::default().fg(ui.theme.yellow),
            ),
            pipe("│", &ui.theme),
            Span::styled(
                format!("{:>17}", format!("rx {} tx {}", i.rx_pps, i.tx_pps)),
                Style::default().fg(ui.theme.fg),
            ),
            pipe("│", &ui.theme),
            Span::styled(
                format!("{:>13}", format!("{} err {} drop", errs, drops)),
                Style::default().fg(bad_color),
            ),
            pipe("│", &ui.theme),
            Span::styled(format!("{:>14}", link), Style::default().fg(ui.theme.fg)),
        ]));
    }
}

/// Processes holding sockets — the ones this user can see, as `ss -p` shows.
fn socket_process_rows(ui: &Ui, inner: Rect, lines: &mut Vec<Line<'static>>) {
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
}

/// Which ports are moving bytes, listening or not. Fullscreen only.
///
/// TCP only, and labelled as such: the kernel keeps no cumulative byte
/// counter for UDP sockets, so QUIC, DNS and WireGuard never show up here.
fn port_traffic_rows(ui: &Ui, focused: bool, lines: &mut Vec<Line<'static>>) {
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
        pipe(&format!("{:>11}", "RX/s"), &ui.theme),
        pipe(&format!("{:>11}", "TX/s"), &ui.theme),
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
                format!("{:>11}", format!("{}/s", short_bytes(p.rx_bps))),
                Style::default().fg(ui.theme.accent),
            ),
            Span::styled(
                format!("{:>11}", format!("{}/s", short_bytes(p.tx_bps))),
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

/// What is listening, and on which port. Fullscreen only.
fn listening_port_rows(ui: &Ui, focused: bool, inner: Rect, lines: &mut Vec<Line<'static>>) {
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
}

pub(super) fn draw_net(frame: &mut Frame, area: Rect, ui: &Ui) {
    let focused = ui.pane == Pane::Net;
    let block = block("3:NET", focused, &ui.theme);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    let totals = &ui.snap.net.totals;

    let mut lines = vec![Line::from(vec![
        pipe(&format!(" {:<9}", "NET"), &ui.theme),
        pipe("│", &ui.theme),
        Span::styled(
            format!("{:>20}", format!("rx {}/s", short_bytes(totals.rx_bps))),
            Style::default().fg(ui.theme.accent),
        ),
        pipe("│", &ui.theme),
        Span::styled(
            format!("{:>20}", format!("tx {}/s", short_bytes(totals.tx_bps))),
            Style::default().fg(ui.theme.yellow),
        ),
        pipe("│", &ui.theme),
        Span::styled(
            format!("{:>17}", format!("tcp retrans {}/s", totals.tcp_retrans_s)),
            Style::default().fg(if totals.tcp_retrans_s > 0 {
                ui.theme.red
            } else {
                ui.theme.fg
            }),
        ),
        pipe("│", &ui.theme),
        Span::styled(
            format!("{:>13}", format!("{} connections", totals.tcp_established)),
            Style::default().fg(ui.theme.fg),
        ),
        pipe("│", &ui.theme),
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
        pipe(&format!(" {:<9}", "IFACE"), &ui.theme),
        pipe("│", &ui.theme),
        pipe(&format!("{:>20}", "RX/s"), &ui.theme),
        pipe("│", &ui.theme),
        pipe(&format!("{:>20}", "TX/s"), &ui.theme),
        pipe("│", &ui.theme),
        pipe(&format!("{:>17}", "rx pps / tx pps"), &ui.theme),
        pipe("│", &ui.theme),
        pipe(&format!("{:>13}", "err / drop"), &ui.theme),
        pipe("│", &ui.theme),
        pipe(&format!("{:>14}", "LINK"), &ui.theme),
    ]));
    interface_rows(ui, &mut lines);
    socket_process_rows(ui, inner, &mut lines);
    port_traffic_rows(ui, focused, &mut lines);
    listening_port_rows(ui, focused, inner, &mut lines);
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_net_summary(frame: &mut Frame, area: Rect, ui: &Ui) {
    crate::tui::net_summary::draw(frame, area, ui);
}

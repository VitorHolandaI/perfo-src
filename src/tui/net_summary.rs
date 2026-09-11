use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::data::net::{NetInfo, NetSnapshot};

use super::cpu::{self, Ui};

const MAX_ACTIVE_INTERFACES: usize = 6;

pub(super) fn draw(frame: &mut Frame, area: ratatui::layout::Rect, ui: &Ui) {
    let panel = cpu::block("3:NET", false, &ui.theme);
    frame.render_widget(panel.clone(), area);
    let inner = panel.inner(area);
    frame.render_widget(
        Paragraph::new(summary_lines(inner.width, inner.height, ui)),
        inner,
    );
}

fn summary_lines(width: u16, height: u16, ui: &Ui) -> Vec<Line<'static>> {
    let n = &ui.snap.net;
    let (errors, drops) = total_errors(n);
    let mut lines = vec![
        totals_line(n, ui),
        session_line(n, ui),
        health_line(errors, drops, n, ui),
    ];
    lines.push(Line::from(Span::styled(
        "ACTIVE INTERFACES (top 6)",
        Style::default().fg(ui.theme.accent),
    )));
    lines.extend(interface_lines(n, height, ui));
    lines.push(ports_line(n, ui));
    lines.push(history_line(width, n, ui));
    lines
}

fn totals_line(n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    Line::from(vec![
        Span::styled("RX ", Style::default().fg(ui.theme.accent)),
        Span::styled(
            format!("{:>8}/s", cpu::short_bytes(n.totals.rx_bps)),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled("    TX ", Style::default().fg(ui.theme.yellow)),
        Span::styled(
            format!("{:>8}/s", cpu::short_bytes(n.totals.tx_bps)),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled(
            format!("    TCP {} established", n.totals.tcp_established),
            Style::default().fg(ui.theme.fg),
        ),
    ])
}

fn health_line(errors: u64, drops: u64, n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    let color = if errors + drops > 0 {
        ui.theme.red
    } else {
        ui.theme.green
    };
    Line::from(vec![Span::styled(
        format!(
            "retrans {:>5}/s  errors {:>4}  drops {:>4}  ports {:>2}",
            n.totals.tcp_retrans_s,
            errors,
            drops,
            n.listening.len()
        ),
        Style::default().fg(color),
    )])
}

fn session_line(n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    Line::from(vec![
        Span::styled("SESSION ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("RX {}", cpu::human_bytes(n.totals.session_rx_bytes)),
            Style::default().fg(ui.theme.accent),
        ),
        Span::styled("  ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            format!("TX {}", cpu::human_bytes(n.totals.session_tx_bytes)),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled("  since monitor start", Style::default().fg(ui.theme.muted)),
    ])
}

fn interface_lines(n: &NetSnapshot, height: u16, ui: &Ui) -> Vec<Line<'static>> {
    let rows = height.saturating_sub(9) as usize;
    let active = active_interfaces(n);
    if active.is_empty() {
        return vec![Line::from(Span::styled(
            "no interface traffic",
            Style::default().fg(ui.theme.muted),
        ))];
    }
    active
        .into_iter()
        .take(rows.min(MAX_ACTIVE_INTERFACES))
        .map(|iface| interface_line(iface, ui))
        .collect()
}

fn active_interfaces(n: &NetSnapshot) -> Vec<&NetInfo> {
    let mut active: Vec<&NetInfo> = n
        .ifaces
        .iter()
        .filter(|i| i.rx_bps > 0 || i.tx_bps > 0)
        .collect();
    active.sort_by_key(|i| std::cmp::Reverse(i.rx_bps.saturating_add(i.tx_bps)));
    active
}

fn interface_line(iface: &NetInfo, ui: &Ui) -> Line<'static> {
    let link = link_label(iface);
    let link_color = if iface.link_up {
        ui.theme.green
    } else {
        ui.theme.muted
    };
    Line::from(vec![
        Span::styled(
            format!("{:<10} ", cpu::truncate(&iface.name, 10)),
            Style::default().fg(ui.theme.fg),
        ),
        Span::styled("RX ", Style::default().fg(ui.theme.accent)),
        Span::styled(
            format!(
                "{:<6} {:>7}/s  ",
                cpu::sparkline(&iface.rx_hist, 6, None),
                cpu::short_bytes(iface.rx_bps)
            ),
            Style::default().fg(ui.theme.accent),
        ),
        Span::styled("TX ", Style::default().fg(ui.theme.yellow)),
        Span::styled(
            format!(
                "{:<6} {:>7}/s  ",
                cpu::sparkline(&iface.tx_hist, 6, None),
                cpu::short_bytes(iface.tx_bps)
            ),
            Style::default().fg(ui.theme.yellow),
        ),
        Span::styled(link, Style::default().fg(link_color)),
    ])
}

fn ports_line(n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    if n.listening.is_empty() {
        return Line::from(Span::styled(
            "ports: none",
            Style::default().fg(ui.theme.muted),
        ));
    }
    let ports = n
        .listening
        .iter()
        .take(4)
        .map(|p| format!("{}/{}", p.port, p.proto))
        .collect::<Vec<_>>()
        .join("  ");
    Line::from(vec![
        Span::styled("PORTS ", Style::default().fg(ui.theme.yellow)),
        Span::styled(ports, Style::default().fg(ui.theme.fg)),
    ])
}

fn history_line(width: u16, n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    let graph_width = width.saturating_sub(32).max(12) as usize / 2;
    Line::from(vec![
        Span::styled("HISTORY RX ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            cpu::sparkline(&n.rx_history, graph_width, None),
            Style::default().fg(ui.theme.accent),
        ),
        Span::styled(" TX ", Style::default().fg(ui.theme.muted)),
        Span::styled(
            cpu::sparkline(&n.tx_history, graph_width, None),
            Style::default().fg(ui.theme.yellow),
        ),
    ])
}

fn total_errors(n: &NetSnapshot) -> (u64, u64) {
    (
        n.ifaces.iter().map(|i| i.rx_errs_s + i.tx_errs_s).sum(),
        n.ifaces.iter().map(|i| i.rx_drops_s + i.tx_drops_s).sum(),
    )
}

fn link_label(iface: &NetInfo) -> String {
    match (iface.link_mbps, iface.link_up) {
        (Some(mbps), true) => format!("UP {mbps}M"),
        (Some(mbps), false) => format!("DOWN {mbps}M"),
        (None, true) => "UP".to_string(),
        (None, false) => "virtual".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn iface(link_mbps: Option<u64>, link_up: bool) -> NetInfo {
        NetInfo {
            name: "eth0".into(),
            rx_bps: 0,
            tx_bps: 0,
            rx_pps: 0,
            tx_pps: 0,
            rx_errs_s: 0,
            tx_errs_s: 0,
            rx_drops_s: 0,
            tx_drops_s: 0,
            link_mbps,
            link_up,
            total_rx_bytes: 0,
            total_tx_bytes: 0,
            rx_hist: VecDeque::new(),
            tx_hist: VecDeque::new(),
        }
    }

    #[test]
    fn link_label_distinguishes_physical_and_virtual_links() {
        assert_eq!(link_label(&iface(Some(1000), true)), "UP 1000M");
        assert_eq!(link_label(&iface(Some(1000), false)), "DOWN 1000M");
        assert_eq!(link_label(&iface(None, false)), "virtual");
    }
}

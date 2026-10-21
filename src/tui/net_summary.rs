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
    let mut lines = vec![totals_line(n, ui), session_line(n, ui), health_line(n, ui)];
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

/// Packets lost across every interface: the lifetime count, how many went
/// missing in this refresh alone, and the worst single interface's share.
///
/// The lifetime count is what answers "is this machine losing packets",
/// which the old per-second-only line could not: a link bleeding all day
/// still reports zero on a quiet second.
fn network_loss(n: &NetSnapshot) -> (u64, u64, f32) {
    let lifetime = n
        .ifaces
        .iter()
        .map(|i| i.session_rx_errs + i.session_rx_drops + i.session_tx_errs + i.session_tx_drops)
        .sum();
    let this_refresh = n
        .ifaces
        .iter()
        .map(|i| i.rx_errs + i.rx_drops + i.tx_errs + i.tx_drops)
        .sum();
    // Only links that actually delivered something get a say in the worst
    // percentage. An idle docker bridge discards every frame it is handed and
    // is honestly at 100%, but letting that set the machine-wide figure buries
    // the link that is really bleeding: this host has eleven such bridges and
    // one wg0 at 5%.
    let worst = n
        .ifaces
        .iter()
        .filter(|i| i.session_rx_packets + i.session_tx_packets > 0)
        .map(|i| i.rx_loss_pct().max(i.tx_loss_pct()))
        .fold(0.0f32, f32::max);
    (lifetime, this_refresh, worst)
}

fn health_line(n: &NetSnapshot, ui: &Ui) -> Line<'static> {
    let (lifetime, this_refresh, worst) = network_loss(n);
    let color = match cpu::loss_level(lifetime, worst) {
        cpu::LossLevel::Clean => ui.theme.green,
        cpu::LossLevel::Minor => ui.theme.yellow,
        cpu::LossLevel::Bad => ui.theme.red,
    };
    Line::from(vec![Span::styled(
        format!(
            "retrans {:>5}/s  lost {:>5} ({:.1}% worst)  now {:>3}  ports {:>2}",
            n.totals.tcp_retrans_s,
            cpu::short_count(lifetime),
            worst,
            this_refresh,
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

    fn iface(link_mbps: Option<u64>, link_up: bool) -> NetInfo {
        NetInfo {
            name: "eth0".into(),
            link_mbps,
            link_up,
            ..NetInfo::default()
        }
    }

    #[test]
    fn link_label_distinguishes_physical_and_virtual_links() {
        assert_eq!(link_label(&iface(Some(1000), true)), "UP 1000M");
        assert_eq!(link_label(&iface(Some(1000), false)), "DOWN 1000M");
        assert_eq!(link_label(&iface(None, false)), "virtual");
    }

    #[test]
    fn network_loss_reports_the_lifetime_count_on_a_quiet_refresh() {
        // wg0 as it stands on this machine: 944 transmit errors on the
        // counter and nothing new in the last second.
        let snap = NetSnapshot {
            ifaces: vec![NetInfo {
                name: "wg0".into(),
                session_tx_packets: 7813,
                session_tx_errs: 944,
                ..NetInfo::default()
            }],
            ..NetSnapshot::default()
        };
        let (lifetime, this_refresh, worst) = network_loss(&snap);
        assert_eq!(lifetime, 944);
        assert_eq!(this_refresh, 0, "nothing new happened");
        assert!((worst - 10.78).abs() < 0.01, "got {worst}");
    }

    #[test]
    fn network_loss_is_silent_on_a_clean_machine() {
        let snap = NetSnapshot {
            ifaces: vec![NetInfo {
                name: "eth0".into(),
                session_rx_packets: 50_000,
                session_tx_packets: 40_000,
                ..NetInfo::default()
            }],
            ..NetSnapshot::default()
        };
        assert_eq!(network_loss(&snap), (0, 0, 0.0));
    }

    #[test]
    fn idle_bridges_do_not_set_the_machine_wide_worst_percentage() {
        // Eleven docker bridges at a literal 100%, and the one link that
        // carries traffic losing 5%. The headline has to be 5%.
        let mut ifaces = vec![NetInfo {
            name: "wg0".into(),
            session_tx_packets: 28_681,
            session_tx_errs: 1_572,
            ..NetInfo::default()
        }];
        ifaces.extend((0..11).map(|n| NetInfo {
            name: format!("br-{n}"),
            session_tx_drops: 1_293,
            ..NetInfo::default()
        }));
        let snap = NetSnapshot {
            ifaces,
            ..NetSnapshot::default()
        };

        let (lifetime, _, worst) = network_loss(&snap);
        assert_eq!(lifetime, 1_572 + 11 * 1_293, "every discard still counts");
        assert!((worst - 5.19).abs() < 0.01, "got {worst}");
    }
}

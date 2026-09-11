use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::data::npu::NpuInfo;

use super::cpu;

pub(super) fn panel_height(devices: &[NpuInfo]) -> u16 {
    if devices.is_empty() {
        0
    } else {
        devices.len().min(3) as u16 + 1
    }
}

pub(super) fn draw(frame: &mut Frame, area: Rect, devices: &[NpuInfo]) {
    let mut lines = vec![Line::from(Span::styled(
        "INTEL NPU                USE   FREQ          MEMORY       PCI",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.extend(
        devices
            .iter()
            .take(area.height.saturating_sub(1) as usize)
            .map(detail_line),
    );
    frame.render_widget(Paragraph::new(lines), area);
}

fn detail_line(device: &NpuInfo) -> Line<'static> {
    Line::from(format!(
        "{:<24} {:>4}  {:<12}  {:>10}   {}",
        cpu::truncate(&device.name, 24),
        percent(device.utilization_percent),
        frequency(device),
        memory(device),
        device.pci_address
    ))
}

fn percent(value: Option<f32>) -> String {
    value
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| "--".into())
}

fn frequency(device: &NpuInfo) -> String {
    match (device.current_frequency_mhz, device.max_frequency_mhz) {
        (Some(current), Some(maximum)) => format!("{current}/{maximum}MHz"),
        _ => "--".into(),
    }
}

fn memory(device: &NpuInfo) -> String {
    device
        .memory_used_bytes
        .map(cpu::short_bytes)
        .unwrap_or_else(|| "--".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> NpuInfo {
        NpuInfo {
            name: "Intel NPU".into(),
            pci_address: "0000:00:0b.0".into(),
            vendor_id: "0x8086".into(),
            device_id: "0x7d1d".into(),
            utilization_percent: Some(42.4),
            current_frequency_mhz: Some(400),
            max_frequency_mhz: Some(1600),
            memory_used_bytes: Some(64 * 1024 * 1024),
        }
    }

    #[test]
    fn lines_format_available_metrics() {
        let device = device();
        assert!(detail_line(&device).to_string().contains("0000:00:0b.0"));
        assert!(detail_line(&device).to_string().contains("42%"));
    }

    #[test]
    fn helpers_keep_unknown_metrics_explicit() {
        let mut device = device();
        device.utilization_percent = None;
        device.current_frequency_mhz = None;
        device.memory_used_bytes = None;
        assert_eq!(percent(device.utilization_percent), "--");
        assert_eq!(frequency(&device), "--");
        assert_eq!(memory(&device), "--");
    }

    #[test]
    fn panel_height_is_bounded() {
        assert_eq!(panel_height(&[]), 0);
        assert_eq!(panel_height(&[device()]), 2);
        assert_eq!(panel_height(&[device(), device(), device(), device()]), 4);
    }
}

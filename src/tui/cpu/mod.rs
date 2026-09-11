//! The dashboard and its fullscreen panes.

mod format;
mod io;
mod net;
mod overlay;
mod panes;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::Paragraph,
    Frame,
};

use crate::data::cpu::{CpuSnapshot, ProcessInfo};
use crate::theme::Theme;

/// Dashboard proportions. The CPU block grows with the core grid; the summary
/// strip under it is fixed, and the remaining width is split between the two
/// detail columns.
const CPU_BLOCK_CHROME: u16 = 6;
const SUMMARY_HEIGHT: u16 = 7;
const LEFT_COLUMN_PCT: u16 = 55;
const RIGHT_COLUMN_PCT: u16 = 45;
const SUMMARY_THIRD_PCT: u16 = 30;
const SUMMARY_MIDDLE_PCT: u16 = 40;

pub(crate) use format::*;
use io::{draw_io, draw_io_summary};
use net::{draw_net, draw_net_summary};
use overlay::{draw_help, draw_menu, draw_status};
use panes::{draw_cpu, draw_disks, draw_mem, draw_process_summary, draw_processes};

/// Cap on rendered core rows (defensive against huge machines).
pub(super) const MAX_CORE_ROWS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Cpu,
    Mem,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Cpu,
    Io,
    Net,
    Mem,
    Disks,
    Gpu,
    History,
}

pub struct Row<'a> {
    pub depth: usize,
    pub process: &'a ProcessInfo,
}

pub struct Ui<'a> {
    pub snap: &'a CpuSnapshot,
    pub rows: &'a [Row<'a>],
    pub selected: Option<usize>,
    pub core_focus: usize,
    pub core_filter: Option<usize>,
    pub sort: SortKey,
    pub invert: bool,
    pub full_cmd: bool,
    pub tree: bool,
    pub pane: Pane,
    pub fullscreen: bool,
    pub theme: Theme,
    pub help: bool,
    pub help_page: usize,
    pub show_menu: bool,
    pub cores_focused: bool,
    pub lang: crate::tui::Lang,
    pub tracing: bool,
    pub trace_lines: Option<&'a std::collections::VecDeque<String>>,
    pub trace_pid: Option<u32>,
    pub history: Option<&'a super::history::HistoryState>,
    pub status: &'a str,
    pub searching: bool,
    pub kill_prompt: bool,
    pub cmd_scroll: usize,
}

pub fn draw(frame: &mut Frame, ui: &Ui) {
    if ui.fullscreen {
        // Focused window owns the whole terminal (status line + help still
        // overlay on top).
        let [body, status_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        match ui.pane {
            Pane::Cpu => draw_cpu_pane(frame, body, ui),
            Pane::Io => draw_io(frame, body, ui),
            Pane::Net => draw_net(frame, body, ui),
            Pane::Mem => super::detail::draw_mem(frame, body, ui),
            Pane::Disks => super::detail::draw_disks(frame, body, ui),
            Pane::Gpu => super::detail::draw_gpu(frame, body, ui),
            Pane::History => {
                if let Some(hist) = ui.history {
                    super::history::draw_history(frame, body, ui, hist);
                }
            }
        }
        draw_status(frame, status_area, ui);
    } else {
        // Dashboard keeps one compact summary of every subsystem visible;
        // `m` or the number shortcuts open a detailed view.
        let core_lines = ui.snap.per_core.len().min(MAX_CORE_ROWS).div_ceil(2);
        let [cpu_area, mid_area, lower_area, status_area] = Layout::vertical([
            Constraint::Length(CPU_BLOCK_CHROME + core_lines as u16),
            Constraint::Length(SUMMARY_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        draw_cpu(frame, cpu_area, ui, true);
        let [mem_area, disk_area, io_area] = Layout::horizontal([
            Constraint::Percentage(SUMMARY_THIRD_PCT),
            Constraint::Percentage(SUMMARY_MIDDLE_PCT),
            Constraint::Percentage(SUMMARY_THIRD_PCT),
        ])
        .areas(mid_area);
        draw_mem(frame, mem_area, ui);
        draw_disks(frame, disk_area, ui);
        draw_io_summary(frame, io_area, ui);
        let [net_area, proc_area] = Layout::horizontal([
            Constraint::Percentage(LEFT_COLUMN_PCT),
            Constraint::Percentage(RIGHT_COLUMN_PCT),
        ])
        .areas(lower_area);
        draw_net_summary(frame, net_area, ui);
        draw_process_summary(frame, proc_area, ui);
        draw_status(frame, status_area, ui);
    }
    if ui.help {
        draw_help(frame, frame.area(), ui);
    }
    if ui.show_menu {
        draw_menu(frame, frame.area(), ui);
    }
}

/// Fullscreen CPU window: one frame around CPU, memory, disks, and processes.
fn draw_cpu_pane(frame: &mut Frame, area: Rect, ui: &Ui) {
    let core_lines = ui.snap.per_core.len().min(MAX_CORE_ROWS).div_ceil(2);
    let title = match ui.core_filter {
        Some(c) => format!("1:CPU + PROCESSES: core {c}"),
        None => "1:CPU + PROCESSES".to_string(),
    };
    let outer = block(&title, true, &ui.theme);
    frame.render_widget(outer.clone(), area);
    let inner = outer.inner(area);
    let [cpu_area, mid_area, proc_area] = Layout::vertical([
        Constraint::Length(CPU_BLOCK_CHROME + core_lines as u16),
        Constraint::Length(SUMMARY_HEIGHT),
        Constraint::Min(0),
    ])
    .areas(inner);
    draw_cpu(frame, cpu_area, ui, false);
    let [mem_area, disk_area, gpu_area] = Layout::horizontal([
        Constraint::Percentage(32),
        Constraint::Percentage(43),
        Constraint::Percentage(25),
    ])
    .areas(mid_area);
    draw_mem(frame, mem_area, ui);
    draw_disks(frame, disk_area, ui);
    super::detail::draw_gpu_summary(frame, gpu_area, ui);
    draw_processes(frame, proc_area, ui, false);
}

pub(super) fn draw_summary(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line>,
    theme: &Theme,
) {
    let panel = block(title, false, theme);
    frame.render_widget(panel.clone(), area);
    frame.render_widget(Paragraph::new(lines), panel.inner(area));
}

#[cfg(test)]
fn metric_card_display(value: Option<f32>, detected: bool) -> String {
    match (value, detected) {
        (Some(percent), _) => format!("{:>3.0}%  {}", percent, bar(percent, 12)),
        (None, true) => "--% (sampling)".into(),
        (None, false) => "not detected".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::io::io_header;
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn short_bytes_scales() {
        assert_eq!(short_bytes(0), "0.0B");
        assert_eq!(short_bytes(1024), "1.0K");
        assert_eq!(short_bytes(20_000), "20K");
        assert_eq!(short_bytes(535_000_000), "510M");
        assert_eq!(short_bytes(8_900_000_000), "8.3G");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(1024), "1.0KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0MiB");
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn truncate_ellipsizes_long_strings() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("日本語テキスト", 3), "日本…");
        assert_eq!(truncate("exactly-nine", 9), "exactly-…");
    }

    #[test]
    fn truncate_with_scroll_offsets_and_ellipsizes() {
        let cmd = "firefox --private-window https://example.com";
        assert_eq!(truncate_with_scroll(cmd, 0, 10), "firefox -…");
        assert_eq!(truncate_with_scroll(cmd, 8, 16), "--private-windo…");
        assert_eq!(truncate_with_scroll(cmd, 25, 30), "https://example.com");
        assert_eq!(truncate_with_scroll(cmd, 100, 10), "");
        assert_eq!(truncate_with_scroll("abc", 0, 0), "");
        assert_eq!(truncate_with_scroll("日本語テキスト", 2, 3), "語テ…");
    }

    #[test]
    fn bar_fills_and_clamps() {
        assert_eq!(bar(50.0, 10), "+++++·····");
        assert_eq!(bar(200.0, 4), "@@@@");
        assert_eq!(bar(0.0, 4), "····");
    }

    #[test]
    fn bar_glyph_scales_from_light_to_heavy() {
        assert_eq!(bar_glyph(0.0), '.');
        assert_eq!(bar_glyph(50.0), '+');
        assert_eq!(bar_glyph(100.0), '@');
    }

    #[test]
    fn io_header_matches_data_cell_width() {
        assert_eq!(io_header("r/s", 7).chars().count(), 7);
        assert_eq!(io_header("r_awt ms", 8).chars().count(), 8);
        assert_eq!(io_header("busy", 6).chars().count(), 6);
        assert_eq!(format!(" {:>7.1}", 1.0).chars().count(), 8);
    }

    #[test]
    fn ghz_format() {
        assert_eq!(ghz(500), "500M");
        assert_eq!(ghz(1000), "1.0G");
        assert_eq!(ghz(4900), "4.9G");
    }

    fn disk(name: &str) -> crate::data::disk::DiskInfo {
        crate::data::disk::DiskInfo {
            name: name.into(),
            mount: "/".into(),
            fs: "btrfs".into(),
            total_bytes: 100,
            available_bytes: 50,
            used_bytes: 50,
            percent: 50.0,
            read_bps: 0,
            write_bps: 0,
            total_read_bytes: 0,
            total_written_bytes: 0,
            temp_c: None,
            io: Default::default(),
        }
    }

    #[test]
    fn unique_disks_dedupes_by_device() {
        let d = disk("/dev/mapper/root");
        let boot = disk("/dev/nvme0n1p1");
        let disks = [d.clone(), d.clone(), boot, d];
        let uniq: Vec<&String> = unique_disks(&disks).iter().map(|x| &x.name).collect();
        assert_eq!(uniq, vec!["/dev/mapper/root", "/dev/nvme0n1p1"]);
    }

    #[test]
    fn unique_disks_caps_at_five() {
        let disks: Vec<_> = (0..8).map(|i| disk(&format!("/dev/disk{i}"))).collect();
        assert_eq!(unique_disks(&disks).len(), 5);
    }

    #[test]
    fn sparkline_buckets_and_scales() {
        let mut q = VecDeque::new();
        for v in [0.0, 0.0, 100.0, 0.0, 0.0, 0.0] {
            q.push_back(v);
        }
        // 6 samples into 2 buckets: [0,0,100] avg 33.3, [0,0,0] avg 0.
        // Absolute scale to 100: 33.3/100*8=2.66→⡆, 0→⡀.
        let s = sparkline(&q, 2, Some(100.0));
        assert_eq!(s.chars().count(), 2);
        assert_eq!(s, "⡆⡀");
        assert_eq!(sparkline(&VecDeque::new(), 3, Some(100.0)), "   ");
    }

    #[test]
    fn sparkline_auto_scales_rates_without_percent_saturation() {
        let q = VecDeque::from([0.0, 842_000_000.0, 0.0]);
        assert_eq!(sparkline(&q, 3, None), "⡀⣿⡀");
    }

    #[test]
    fn sparkline_handles_zero_width() {
        let samples = VecDeque::from([10.0, 20.0]);
        assert_eq!(sparkline(&samples, 0, None), "");
    }

    #[test]
    fn temp_color_thresholds() {
        let t = Theme::DEFAULT;
        assert_eq!(temp_color(Some(40.0), &t), t.green);
        assert_eq!(temp_color(Some(60.0), &t), t.yellow);
        assert_eq!(temp_color(Some(80.0), &t), t.red);
        assert_eq!(temp_color(None, &t), t.green);
    }

    #[test]
    fn metric_card_display_formats_values_and_distinguishes_sampling_from_missing() {
        assert_eq!(metric_card_display(Some(50.0), true), " 50%  ++++++······");
        assert_eq!(metric_card_display(None, true), "--% (sampling)");
        assert_eq!(metric_card_display(None, false), "not detected");
    }
}

//! The terminal user interface.

mod app;
mod clipboard;
pub mod cpu;
mod detail;
mod help;
pub mod history;
mod keys;
mod net_summary;
mod npu;
mod status;

pub use app::{run, run_with_pane};

use std::time::Duration;

use cpu::{Pane, SortKey};

const TICK: Duration = Duration::from_millis(1000);
/// Trace backlog kept in memory for the TUI trace pane.
const TRACE_LINES_MAX: usize = 300;
/// Last help page index (6 pages, 0-based).
const HELP_LAST_PAGE: usize = 5;
/// Rows jumped per PageUp/PageDown in the process table.
const PAGE_STEP: i32 = 10;

/// UI language for help text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    Pt,
}

struct State {
    selected_pid: Option<u32>,
    sort: SortKey,
    invert: bool,
    core_focus: usize,
    core_filter: Option<usize>,
    full_cmd: bool,
    tree: bool,
    show_threads: bool,
    show_kernel: bool,
    search: String,
    searching: bool,
    kill_prompt: bool,
    status_msg: Option<String>,
    pane: Pane,
    /// Fullscreen mode: only `pane` renders (numbers expand each pane).
    /// False = dashboard aggregates CPU + mem + disks + the active pane.
    fullscreen: bool,
    paused: bool,
    use_system_theme: bool,
    lang: Lang,
    help: bool,
    help_page: usize,
    show_menu: bool,
    /// Which section receives arrow-key navigation in the unified CPU view.
    cores_focused: bool,
    tracing: bool,
    trace_start_pid: Option<u32>,
    pub history: history::HistoryState,
    cmd_scroll: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            selected_pid: None,
            sort: SortKey::Cpu,
            invert: true,
            core_focus: 0,
            core_filter: None,
            full_cmd: false,
            tree: false,
            show_threads: false,
            show_kernel: false,
            search: String::new(),
            searching: false,
            kill_prompt: false,
            status_msg: None,
            pane: Pane::Cpu,
            fullscreen: false,
            paused: false,
            use_system_theme: true,
            lang: Lang::default(),
            help: false,
            help_page: 0,
            show_menu: false,
            cores_focused: false,
            tracing: false,
            trace_start_pid: None,
            history: history::HistoryState::default(),
            cmd_scroll: 0,
        }
    }
}

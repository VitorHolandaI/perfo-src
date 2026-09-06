pub mod cpu;
mod detail;
mod help;
mod net_summary;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{init, restore};

use crate::data::cpu::{CpuMonitor, CpuSnapshot, ProcessInfo};
use crate::theme::{self, Theme};
use crate::trace;
use cpu::{Pane, Row, SortKey, Ui};

const TICK: Duration = Duration::from_millis(1000);
/// Trace backlog kept in memory for the TUI trace pane.
const TRACE_LINES_MAX: usize = 300;
/// Last help page index (5 pages, 0-based).
const HELP_LAST_PAGE: usize = 4;
/// Rows jumped per PageUp/PageDown in the process table.
const PAGE_STEP: i32 = 10;

pub fn run() -> std::io::Result<()> {
    let mut terminal = init();
    let mut monitor = CpuMonitor::new();
    let result = run_loop(&mut terminal, &mut monitor);
    restore();
    result
}

/// UI language for help text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Pt,
    En,
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
            lang: Lang::En,
            help: false,
            help_page: 0,
            show_menu: false,
            cores_focused: true,
            tracing: false,
            trace_start_pid: None,
        }
    }
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    monitor: &mut CpuMonitor,
) -> std::io::Result<()> {
    let mut state = State::default();
    let mut display_pids: Vec<u32> = Vec::new();
    let mut snap: Option<CpuSnapshot> = None;
    let mut last_tick = Instant::now() - TICK;
    let mut full_tick = false;
    let system_theme = theme::system();

    let (trace_tx, trace_rx) = mpsc::channel::<String>();
    let mut trace_lines: VecDeque<String> = VecDeque::new();
    let mut trace_thread: Option<std::thread::JoinHandle<()>> = None;

    loop {
        if last_tick.elapsed() >= TICK && !state.paused {
            // Process stats are the expensive part; refresh them every other
            // tick so the bars stay at 1s while the table lags 2s.
            if full_tick {
                monitor.refresh();
            } else {
                monitor.refresh_light();
            }
            full_tick = !full_tick;
            snap = Some(monitor.snapshot());
            last_tick = Instant::now();
        }

        let wait = if last_tick.elapsed() >= TICK {
            Duration::ZERO
        } else {
            TICK.saturating_sub(last_tick.elapsed())
        };
        if event::poll(wait)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && handle_key(
                        &mut state,
                        &display_pids,
                        key.code,
                        key.modifiers,
                        system_theme,
                    )
                {
                    break;
                }
            }
        }

        // Manage the tracer thread.
        manage_trace_thread(
            &mut state,
            &mut trace_thread,
            &trace_tx,
            &trace_rx,
            &mut trace_lines,
        );

        if let Some(s) = &snap {
            let (rows, pids, selected) = prepare(s, &mut state);
            display_pids = pids;
            let status = status_line(&state);
            let theme = if state.use_system_theme {
                system_theme.unwrap_or(Theme::DEFAULT)
            } else {
                Theme::DEFAULT
            };
            let ui = Ui {
                snap: s,
                rows: &rows,
                selected,
                core_focus: state.core_focus,
                core_filter: state.core_filter,
                sort: state.sort,
                invert: state.invert,
                full_cmd: state.full_cmd,
                tree: state.tree,
                pane: state.pane,
                fullscreen: state.fullscreen,
                theme,
                help: state.help,
                help_page: state.help_page,
                show_menu: state.show_menu,
                cores_focused: state.cores_focused,
                lang: state.lang,
                tracing: state.tracing || trace_thread.is_some(),
                trace_lines: if state.tracing || trace_thread.is_some() {
                    Some(&trace_lines)
                } else {
                    None
                },
                trace_pid: state.trace_start_pid,
                status: &status,
                searching: state.searching,
                kill_prompt: state.kill_prompt,
            };
            terminal.draw(|frame| cpu::draw(frame, &ui))?;
        }
    }
    Ok(())
}

fn prepare<'a>(
    snap: &'a CpuSnapshot,
    state: &mut State,
) -> (Vec<Row<'a>>, Vec<u32>, Option<usize>) {
    if !snap.per_core.is_empty() {
        state.core_focus = state.core_focus.min(snap.per_core.len() - 1);
    }
    let needle = state.search.to_lowercase();
    let filtered: Vec<&'a ProcessInfo> = snap
        .processes
        .iter()
        .filter(|p| state.show_kernel || !p.is_kernel)
        .filter(|p| state.show_threads || p.owner.is_none())
        .filter(|p| {
            state
                .core_filter
                .is_none_or(|c| p.last_cpu == Some(c as u32))
        })
        .filter(|p| needle.is_empty() || p.cmd.to_lowercase().contains(&needle))
        .collect();

    let rows = if state.tree {
        build_tree(&filtered, state.sort, state.invert)
    } else {
        let mut v = filtered;
        sort_procs(&mut v, state.sort, state.invert);
        v.into_iter()
            .map(|p| Row {
                depth: 0,
                process: p,
            })
            .collect()
    };
    let display_pids: Vec<u32> = rows.iter().map(|r| r.process.pid).collect();
    let selected = state
        .selected_pid
        .and_then(|pid| rows.iter().position(|r| r.process.pid == pid));
    (rows, display_pids, selected)
}

fn sort_procs(v: &mut [&ProcessInfo], sort: SortKey, desc: bool) {
    let key = |p: &ProcessInfo| match sort {
        SortKey::Cpu => p.cpu_percent.to_bits() as u64,
        SortKey::Mem => p.mem_bytes,
    };
    if desc {
        v.sort_by_key(|p| std::cmp::Reverse(key(p)));
    } else {
        v.sort_by_key(|p| key(p));
    }
}

fn build_tree<'a>(procs: &[&'a ProcessInfo], sort: SortKey, desc: bool) -> Vec<Row<'a>> {
    let present: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
    let mut children: HashMap<Option<u32>, Vec<&'a ProcessInfo>> = HashMap::new();
    for p in procs {
        let parent = p.owner.or(p.ppid).filter(|pp| present.contains(pp));
        children.entry(parent).or_default().push(p);
    }
    let mut roots = children.remove(&None).unwrap_or_default();
    sort_procs(&mut roots, sort, desc);

    fn walk<'a>(
        pid: u32,
        depth: usize,
        children: &HashMap<Option<u32>, Vec<&'a ProcessInfo>>,
        sort: SortKey,
        desc: bool,
        out: &mut Vec<Row<'a>>,
    ) {
        if let Some(kids) = children.get(&Some(pid)) {
            let mut kids = kids.clone();
            sort_procs(&mut kids, sort, desc);
            for k in kids {
                out.push(Row { depth, process: k });
                walk(k.pid, depth + 1, children, sort, desc, out);
            }
        }
    }

    let mut out = Vec::new();
    for r in roots {
        out.push(Row {
            depth: 0,
            process: r,
        });
        walk(r.pid, 1, &children, sort, desc, &mut out);
    }
    out
}

fn manage_trace_thread(
    state: &mut State,
    trace_thread: &mut Option<std::thread::JoinHandle<()>>,
    trace_tx: &mpsc::Sender<String>,
    trace_rx: &mpsc::Receiver<String>,
    trace_lines: &mut VecDeque<String>,
) {
    if state.tracing && trace_thread.is_none() {
        if let Some(pid) = state.trace_start_pid {
            let tx2 = trace_tx.clone();
            *trace_thread = Some(std::thread::spawn(move || {
                if let Err(e) = trace::attach_stream(pid as i32, None, tx2.clone()) {
                    let _ = tx2.send(format!("perfo trace: {e}"));
                }
            }));
        } else {
            state.tracing = false;
        }
    } else if let Some(th) = trace_thread {
        if th.is_finished() || !state.tracing {
            trace::stop_current_trace();
            let _ = trace_thread.take().unwrap().join();
            state.tracing = false;
            state.trace_start_pid = None;
            state.status_msg = match trace_lines.back() {
                Some(last) if last.starts_with("perfo trace:") => Some(last.clone()),
                _ => Some("trace ended".into()),
            };
        }
    }
    while let Ok(line) = trace_rx.try_recv() {
        trace_lines.push_back(line);
        if trace_lines.len() > TRACE_LINES_MAX {
            trace_lines.pop_front();
        }
    }
}

fn handle_key(
    state: &mut State,
    display_pids: &[u32],
    code: KeyCode,
    mods: KeyModifiers,
    system_theme: Option<Theme>,
) -> bool {
    if state.help {
        handle_help_key(state, code);
        return false;
    }
    if state.show_menu {
        handle_menu_key(state, code);
        return false;
    }
    if state.tracing {
        handle_tracing_key(state, code);
        return false;
    }
    if code == KeyCode::Char('c') && mods.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if state.searching {
        handle_searching_key(state, code);
        return false;
    }
    if state.kill_prompt {
        handle_kill_key(state, code);
        return false;
    }
    handle_normal_key(state, display_pids, code, system_theme)
}

fn handle_help_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::PageDown | KeyCode::Char('n') | KeyCode::Char('j') => {
            state.help_page = (state.help_page + 1).min(HELP_LAST_PAGE);
        }
        KeyCode::PageUp | KeyCode::Char('p') | KeyCode::Char('k') => {
            state.help_page = state.help_page.saturating_sub(1);
        }
        KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            state.help = false;
        }
        KeyCode::Char('L') => {
            toggle_lang(state);
        }
        _ => {}
    }
}

fn handle_tracing_key(state: &mut State, code: KeyCode) {
    if matches!(
        code,
        KeyCode::Char('s')
            | KeyCode::Char('S')
            | KeyCode::Char('q')
            | KeyCode::Char('Q')
            | KeyCode::Esc
    ) {
        state.tracing = false;
    }
}

fn handle_searching_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Char(c) if c.is_ascii_graphic() || c == ' ' => state.search.push(c),
        KeyCode::Backspace => {
            state.search.pop();
        }
        KeyCode::Esc => {
            state.searching = false;
            state.search.clear();
        }
        KeyCode::Enter => {
            state.searching = false;
        }
        _ => {}
    }
}

fn handle_kill_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Char('1') => {
            send_signal(state, libc::SIGTERM);
        }
        KeyCode::Char('9') => {
            send_signal(state, libc::SIGKILL);
        }
        KeyCode::Char('0') | KeyCode::Esc => {}
        _ => return,
    }
    state.kill_prompt = false;
}

fn handle_normal_key(
    state: &mut State,
    display_pids: &[u32],
    code: KeyCode,
    system_theme: Option<Theme>,
) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => true,
        KeyCode::Esc => {
            // Esc clears the core filter first; a second Esc quits.
            if state.core_filter.is_some() {
                state.core_filter = None;
                false
            } else {
                true
            }
        }
        KeyCode::Tab | KeyCode::BackTab => {
            if state.pane == Pane::Cpu {
                state.cores_focused = !state.cores_focused;
            }
            false
        }
        KeyCode::Char('k') => {
            if state.selected_pid.is_some() {
                state.status_msg = None;
                state.kill_prompt = true;
            }
            false
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            state.sort = SortKey::Cpu;
            false
        }
        KeyCode::Char('M') => {
            state.sort = SortKey::Mem;
            false
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            state.invert = !state.invert;
            false
        }
        KeyCode::Char('c') => {
            state.full_cmd = !state.full_cmd;
            false
        }
        KeyCode::Char('t') => {
            state.tree = !state.tree;
            false
        }
        KeyCode::Char('h') | KeyCode::Char('H') => {
            state.show_threads = !state.show_threads;
            false
        }
        KeyCode::Char('K') => {
            state.show_kernel = !state.show_kernel;
            false
        }
        KeyCode::Char('1') => {
            focus_pane(state, Pane::Cpu);
            false
        }
        KeyCode::Char('2') => {
            focus_pane(state, Pane::Io);
            false
        }
        KeyCode::Char('3') => {
            focus_pane(state, Pane::Net);
            false
        }
        KeyCode::Char('4') => {
            focus_pane(state, Pane::Mem);
            false
        }
        KeyCode::Char('5') => {
            focus_pane(state, Pane::Disks);
            false
        }
        KeyCode::Char('6') => {
            focus_pane(state, Pane::Gpu);
            false
        }
        KeyCode::Char('C') => toggle_theme(state, system_theme),
        KeyCode::Char('L') => toggle_lang(state),
        KeyCode::Char('?') => {
            state.help = true;
            false
        }
        KeyCode::Char('m') => {
            state.show_menu = !state.show_menu;
            false
        }
        KeyCode::Char('s') => start_trace(state),
        KeyCode::Char('z') | KeyCode::Char('Z') => {
            state.paused = !state.paused;
            false
        }
        KeyCode::Char('/') => {
            state.searching = true;
            false
        }
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End => handle_nav_key(state, display_pids, code),
        KeyCode::Left | KeyCode::Right => {
            if state.pane == Pane::Cpu && state.cores_focused {
                move_core(state, code);
            }
            false
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if state.cores_focused {
                toggle_core_filter(state);
            }
            false
        }
        _ => false,
    }
}

fn handle_menu_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Char('m') | KeyCode::Esc => state.show_menu = false,
        KeyCode::Char('1') => select_menu_pane(state, Pane::Cpu),
        KeyCode::Char('2') => select_menu_pane(state, Pane::Io),
        KeyCode::Char('3') => select_menu_pane(state, Pane::Net),
        KeyCode::Char('4') => select_menu_pane(state, Pane::Mem),
        KeyCode::Char('5') => select_menu_pane(state, Pane::Disks),
        KeyCode::Char('6') => select_menu_pane(state, Pane::Gpu),
        _ => {}
    }
}

fn select_menu_pane(state: &mut State, pane: Pane) {
    state.pane = pane;
    state.fullscreen = true;
    state.show_menu = false;
}

/// Expand `target` fullscreen, or drop back to the dashboard when the same
/// pane's number is pressed again.
fn focus_pane(state: &mut State, target: Pane) {
    if state.fullscreen && state.pane == target {
        state.fullscreen = false;
    } else {
        state.pane = target;
        state.fullscreen = true;
    }
}

fn toggle_lang(state: &mut State) -> bool {
    state.lang = match state.lang {
        Lang::Pt => Lang::En,
        Lang::En => Lang::Pt,
    };
    state.status_msg = Some(match state.lang {
        Lang::Pt => "idioma: PT".into(),
        Lang::En => "language: EN".into(),
    });
    false
}

fn toggle_theme(state: &mut State, system_theme: Option<Theme>) -> bool {
    if state.use_system_theme {
        state.use_system_theme = false;
        state.status_msg = Some("theme: default".into());
    } else if system_theme.is_some() {
        state.use_system_theme = true;
        state.status_msg = Some("theme: omarchy".into());
    } else {
        state.status_msg = Some("no system theme found".into());
    }
    false
}

fn start_trace(state: &mut State) -> bool {
    if let Some(pid) = state.selected_pid {
        state.status_msg = None;
        state.tracing = true;
        state.trace_start_pid = Some(pid);
    }
    false
}

fn toggle_core_filter(state: &mut State) -> bool {
    if state.pane == Pane::Cpu {
        state.core_filter = match state.core_filter {
            Some(c) if c == state.core_focus => None,
            _ => Some(state.core_focus),
        };
    }
    false
}

fn handle_nav_key(state: &mut State, display_pids: &[u32], code: KeyCode) -> bool {
    if state.pane != Pane::Cpu {
        return false;
    }
    if state.cores_focused {
        move_core(state, code);
        return false;
    }
    let delta = match code {
        KeyCode::Up => -1,
        KeyCode::Down => 1,
        KeyCode::PageUp => -PAGE_STEP,
        KeyCode::PageDown => PAGE_STEP,
        KeyCode::Home => i32::MIN,
        KeyCode::End => i32::MAX,
        _ => return false,
    };
    move_selection(state, display_pids, delta);
    false
}

fn move_core(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Left => state.core_focus = state.core_focus.saturating_sub(1),
        KeyCode::Right => state.core_focus += 1,
        KeyCode::Up => state.core_focus = state.core_focus.saturating_sub(2),
        KeyCode::Down => state.core_focus += 2,
        _ => {}
    }
}

fn move_selection(state: &mut State, display_pids: &[u32], delta: i32) {
    if display_pids.is_empty() {
        return;
    }
    let idx = state
        .selected_pid
        .and_then(|pid| display_pids.iter().position(|p| *p == pid))
        .unwrap_or(0);
    let ni = (idx as i64 + delta as i64).clamp(0, display_pids.len() as i64 - 1) as usize;
    state.selected_pid = Some(display_pids[ni]);
    state.status_msg = None;
}

fn send_signal(state: &mut State, sig: i32) {
    if let Some(pid) = state.selected_pid {
        // SAFETY: pid came from our own process table (live at selection
        // time) and the user explicitly confirmed the signal in the prompt.
        let r = unsafe { libc::kill(pid as i32, sig) };
        state.status_msg = Some(if r == 0 {
            format!("sent signal {sig} to {pid}")
        } else {
            format!("kill {pid} failed: {}", std::io::Error::last_os_error())
        });
    }
}

fn status_line(state: &State) -> String {
    if state.searching {
        return format!("/{}{}", state.search, "_");
    }
    if state.kill_prompt {
        if let Some(pid) = state.selected_pid {
            return match state.lang {
                Lang::Pt => format!("matar {pid}?  1=SIGTERM  9=SIGKILL  0=cancela"),
                Lang::En => format!("kill {pid}?  1=SIGTERM  9=SIGKILL  0=cancel"),
            };
        }
    }
    if state.tracing {
        if let Some(pid) = state.trace_start_pid {
            return match state.lang {
                Lang::Pt => format!("TRACE {pid} — s/q para parar (syscalls ao vivo)"),
                Lang::En => format!("TRACING {pid} — s/q to stop (live syscalls)"),
            };
        }
    }
    if let Some(msg) = &state.status_msg {
        return msg.clone();
    }
    let pane = match state.pane {
        Pane::Cpu => {
            if state.cores_focused {
                "[1:CPU + PROCS | CORES] "
            } else {
                "[1:CPU + PROCS | PROCESSES] "
            }
        }
        Pane::Io => "[2:IO] ",
        Pane::Net => "[3:NET] ",
        Pane::Mem => "[4:MEM] ",
        Pane::Disks => "[5:DISKS] ",
        Pane::Gpu => "[6:GPU] ",
    };
    let full = if state.fullscreen { "[FULL] " } else { "" };
    let filter = state
        .core_filter
        .map(|c| format!(" core filter {c} | Esc clears |"))
        .unwrap_or_default();
    format!(
        "{pane}{full}{}m menu | Tab focus | {filter} q quit | k kill | arrows navigate | Enter filter core | p CPU | M MEM | i reverse | c cmd | t tree{} | H threads{} | K kernel{} | s trace | z pause | / search | L language | ? help",
        if state.paused { "\u{23F8} PAUSED " } else { "" },
        if state.tree { " \u{2713}" } else { "" },
        if state.show_threads { " \u{2713}" } else { "" },
        if state.show_kernel { " \u{2713}" } else { "" },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::cpu::ProcessInfo;
    use crossterm::event::KeyCode;

    fn proc(pid: u32, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: format!("p{pid}"),
            ppid: None,
            owner: None,
            is_kernel: false,
            user: "u".into(),
            cpu_percent: cpu,
            mem_bytes: mem,
            cmd: format!("p{pid}"),
            last_cpu: None,
            read_bps: 0,
            write_bps: 0,
            win_read_bytes: 0,
            win_write_bytes: 0,
        }
    }

    fn proc_with(pid: u32, ppid: Option<u32>) -> ProcessInfo {
        ProcessInfo {
            ppid,
            ..proc(pid, 0.0, 0)
        }
    }

    #[test]
    fn sort_procs_cpu_desc() {
        let p1 = proc(1, 10.0, 100);
        let p2 = proc(2, 50.0, 50);
        let mut v = [&p1, &p2];
        sort_procs(&mut v, SortKey::Cpu, true);
        assert_eq!(v[0].pid, 2);
    }

    #[test]
    fn sort_procs_mem_asc() {
        let p1 = proc(1, 10.0, 100);
        let p2 = proc(2, 50.0, 50);
        let mut v = [&p1, &p2];
        sort_procs(&mut v, SortKey::Mem, false);
        assert_eq!(v[0].pid, 2);
    }

    #[test]
    fn build_tree_nests_children() {
        let p1 = proc_with(1, None);
        let p2 = proc_with(2, Some(1));
        let p3 = proc_with(3, Some(2));
        let rows = build_tree(&[&p1, &p2, &p3], SortKey::Cpu, false);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].depth, 2);
    }

    #[test]
    fn build_tree_sorts_roots() {
        let slow = proc_with(1, None);
        let fast = proc_with(2, None);
        // Equal cpu keys keep stable input order (sort is by key, stable).
        assert_eq!(
            build_tree(&[&slow, &fast], SortKey::Cpu, true)[0]
                .process
                .pid,
            1
        );
        assert_eq!(
            build_tree(&[&fast, &slow], SortKey::Cpu, true)[0]
                .process
                .pid,
            2
        );
    }

    #[test]
    fn move_core_arrows() {
        let mut s = State::default();
        move_core(&mut s, KeyCode::Right);
        assert_eq!(s.core_focus, 1);
        move_core(&mut s, KeyCode::Up);
        assert_eq!(s.core_focus, 0);
        move_core(&mut s, KeyCode::Down);
        assert_eq!(s.core_focus, 2);
    }

    #[test]
    fn move_selection_clamps() {
        let pids = [10, 20, 30];
        let mut s = State::default();
        move_selection(&mut s, &pids, 1);
        assert_eq!(s.selected_pid, Some(20));
        move_selection(&mut s, &pids, i32::MAX);
        assert_eq!(s.selected_pid, Some(30));
        move_selection(&mut s, &pids, i32::MIN);
        assert_eq!(s.selected_pid, Some(10));
    }

    #[test]
    fn move_selection_empty_is_noop() {
        let mut s = State::default();
        move_selection(&mut s, &[], 1);
        assert_eq!(s.selected_pid, None);
    }

    #[test]
    fn keys_toggle_sort_pause_theme() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('M'), KeyModifiers::empty(), None);
        assert_eq!(s.sort, SortKey::Mem);
        handle_key(&mut s, &[], KeyCode::Char('p'), KeyModifiers::empty(), None);
        assert_eq!(s.sort, SortKey::Cpu);
        handle_key(&mut s, &[], KeyCode::Char('z'), KeyModifiers::empty(), None);
        assert!(s.paused);
        handle_key(&mut s, &[], KeyCode::Char('C'), KeyModifiers::empty(), None);
        assert!(!s.use_system_theme);
        assert_eq!(s.status_msg.as_deref(), Some("theme: default"));
    }

    #[test]
    fn pane_numbers_focus_and_toggle_back() {
        let mut s = State::default();
        assert_eq!(s.pane, Pane::Cpu);
        assert!(!s.fullscreen);
        // 1 expande CPU em tela cheia; repetir 1 volta pro dashboard.
        handle_key(&mut s, &[], KeyCode::Char('1'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Cpu);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('1'), KeyModifiers::empty(), None);
        assert!(!s.fullscreen);
        assert_eq!(s.pane, Pane::Cpu);
        // 2 expande IO; 3 troca o painel expandido; 3 de novo volta.
        handle_key(&mut s, &[], KeyCode::Char('2'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Io);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('3'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Net);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('3'), KeyModifiers::empty(), None);
        assert!(!s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('4'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Mem);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('5'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Disks);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('6'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Gpu);
        assert!(s.fullscreen);
    }

    #[test]
    fn lang_toggles_between_pt_and_en() {
        let mut s = State::default();
        assert_eq!(s.lang, Lang::En);
        handle_key(&mut s, &[], KeyCode::Char('L'), KeyModifiers::empty(), None);
        assert_eq!(s.lang, Lang::Pt);
        handle_key(&mut s, &[], KeyCode::Char('L'), KeyModifiers::empty(), None);
        assert_eq!(s.lang, Lang::En);
        assert!(s.status_msg.is_some());
    }

    #[test]
    fn help_navigates_pages_and_closes() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('?'), KeyModifiers::empty(), None);
        assert!(s.help);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        assert_eq!(s.help_page, 1);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        assert_eq!(s.help_page, HELP_LAST_PAGE);
        handle_key(&mut s, &[], KeyCode::Char('p'), KeyModifiers::empty(), None);
        assert_eq!(s.help_page, 3);
        handle_key(&mut s, &[], KeyCode::Char('q'), KeyModifiers::empty(), None);
        assert!(!s.help);
    }

    #[test]
    fn search_typing_and_escape() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('/'), KeyModifiers::empty(), None);
        assert!(s.searching);
        handle_key(&mut s, &[], KeyCode::Char('a'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('b'), KeyModifiers::empty(), None);
        assert_eq!(s.search, "ab");
        handle_key(&mut s, &[], KeyCode::Esc, KeyModifiers::empty(), None);
        assert!(!s.searching);
        assert!(s.search.is_empty());
    }

    #[test]
    fn esc_clears_core_filter_then_quits() {
        let mut s = State {
            core_filter: Some(2),
            ..State::default()
        };
        assert!(!handle_key(
            &mut s,
            &[],
            KeyCode::Esc,
            KeyModifiers::empty(),
            None
        ));
        assert_eq!(s.core_filter, None);
        assert!(handle_key(
            &mut s,
            &[],
            KeyCode::Esc,
            KeyModifiers::empty(),
            None
        ));
    }

    #[test]
    fn q_quits_ctrl_c_quits() {
        let mut s = State::default();
        assert!(handle_key(
            &mut s,
            &[],
            KeyCode::Char('q'),
            KeyModifiers::empty(),
            None
        ));
        assert!(handle_key(
            &mut s,
            &[],
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            None
        ));
    }

    #[test]
    fn kill_prompt_requires_selection() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('k'), KeyModifiers::empty(), None);
        assert!(!s.kill_prompt);
        s.selected_pid = Some(99999);
        handle_key(&mut s, &[], KeyCode::Char('k'), KeyModifiers::empty(), None);
        assert!(s.kill_prompt);
        // '0' cancels without sending a signal.
        handle_key(&mut s, &[], KeyCode::Char('0'), KeyModifiers::empty(), None);
        assert!(!s.kill_prompt);
    }

    #[test]
    fn status_line_shows_context() {
        let mut s = State::default();
        assert!(status_line(&s).contains("[1:CPU + PROCS | CORES]"));
        s.paused = true;
        assert!(status_line(&s).contains("PAUSED"));
        s.core_filter = Some(3);
        assert!(status_line(&s).contains("core filter 3"));
    }

    #[test]
    fn tab_switches_cpu_subfocus_without_changing_panel() {
        let mut s = State::default();
        assert!(s.cores_focused);
        handle_key(&mut s, &[], KeyCode::Tab, KeyModifiers::empty(), None);
        assert!(!s.cores_focused);
        assert_eq!(s.pane, Pane::Cpu);
        handle_key(&mut s, &[], KeyCode::Tab, KeyModifiers::empty(), None);
        assert!(s.cores_focused);
    }

    #[test]
    fn menu_selects_panel_and_closes() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('m'), KeyModifiers::empty(), None);
        assert!(s.show_menu);
        handle_key(&mut s, &[], KeyCode::Char('3'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Net);
        assert!(s.fullscreen);
        assert!(!s.show_menu);
        handle_key(&mut s, &[], KeyCode::Char('m'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('4'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Mem);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('m'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('6'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::Gpu);
        assert!(s.fullscreen);
    }

    #[test]
    fn tracing_key_stops_on_s_or_q() {
        let mut s = State {
            tracing: true,
            ..State::default()
        };
        handle_key(&mut s, &[], KeyCode::Char('x'), KeyModifiers::empty(), None);
        assert!(s.tracing, "other keys must not stop the trace");
        handle_key(&mut s, &[], KeyCode::Char('s'), KeyModifiers::empty(), None);
        assert!(!s.tracing);
    }
}

//! The terminal event loop and the per-tick data preparation.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{init, restore};

use crate::data::cpu::{CollectionPlan, CollectionProfile, CpuMonitor, CpuSnapshot, ProcessInfo};
use crate::theme::{self, Theme};
use crate::trace;

use super::cpu::{self, Pane, Row, SortKey, Ui};
use super::keys::handle_key;
use super::status::status_line_for_width;
use super::{State, TICK, TRACE_LINES_MAX};

pub fn run() -> std::io::Result<()> {
    run_with_pane(Pane::Cpu)
}

pub fn run_with_pane(pane: Pane) -> std::io::Result<()> {
    let mut terminal = init();
    let mut state = State::default();
    if pane != Pane::Cpu {
        state.pane = pane;
        state.fullscreen = true;
    }
    let mut monitor = CpuMonitor::new_for(collection_plan(&state));
    let result = run_loop(&mut terminal, &mut monitor, state);
    restore();
    result
}

fn visible_profile(state: &State) -> CollectionProfile {
    if !state.fullscreen {
        return CollectionProfile::Dashboard;
    }
    match state.pane {
        Pane::Cpu => CollectionProfile::Cpu,
        Pane::Io => CollectionProfile::Io,
        Pane::Net => CollectionProfile::Net,
        Pane::Mem => CollectionProfile::Mem,
        Pane::Disks => CollectionProfile::Disks,
        Pane::Gpu => CollectionProfile::Gpu,
        Pane::History => CollectionProfile::History,
    }
}

fn collection_plan(state: &State) -> CollectionPlan {
    CollectionPlan::with_recording_mask(
        visible_profile(state),
        state.history.is_session_recording,
        state.history.recording_mask,
    )
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    monitor: &mut CpuMonitor,
    mut state: State,
) -> std::io::Result<()> {
    let mut display_pids: Vec<u32> = Vec::new();
    let mut snap: Option<CpuSnapshot> = None;
    let mut last_tick = Instant::now() - TICK;
    let mut full_tick = false;
    let mut active_plan = collection_plan(&state);
    let mut force_refresh = false;
    let system_theme = theme::system();

    let (trace_tx, trace_rx) = mpsc::channel::<String>();
    let mut trace_lines: VecDeque<String> = VecDeque::new();
    let mut trace_thread: Option<std::thread::JoinHandle<()>> = None;

    loop {
        if last_tick.elapsed() >= TICK && (!state.paused || force_refresh) {
            // Process stats are the expensive part; refresh them every other
            // tick so the bars stay at 1s while the table lags 2s.
            monitor.refresh_for(active_plan, full_tick);
            full_tick = !full_tick;
            let s = monitor.snapshot_for(active_plan);
            if active_plan.recording || active_plan.visible == CollectionProfile::History {
                state.history.record_snapshot(&s);
                state.history.advance_playback();
            }
            snap = Some(s);
            last_tick = Instant::now();
            force_refresh = false;
        }

        let wait = if state.paused && !force_refresh {
            TICK
        } else if last_tick.elapsed() >= TICK {
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

        let requested_plan = collection_plan(&state);
        if requested_plan != active_plan {
            active_plan = requested_plan;
            full_tick = true;
            force_refresh = true;
            last_tick = Instant::now() - TICK;
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
            let (rows, pids, selected) = if active_plan.visible == CollectionProfile::Cpu
                || active_plan.visible == CollectionProfile::Dashboard
            {
                prepare(s, &mut state)
            } else {
                (Vec::new(), Vec::new(), None)
            };
            display_pids = pids;
            let term_width = terminal.size().map(|s| s.width as usize).unwrap_or(0);
            let status = status_line_for_width(&state, term_width);
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
                show_threads: state.show_threads,
                lang: state.lang,
                tracing: state.tracing || trace_thread.is_some(),
                trace_lines: if state.tracing || trace_thread.is_some() {
                    Some(&trace_lines)
                } else {
                    None
                },
                trace_pid: state.trace_start_pid,
                history: Some(&state.history),
                status: &status,
                searching: state.searching,
                kill_prompt: state.kill_prompt,
                cmd_scroll: state.cmd_scroll,
            };
            terminal.draw(|frame| cpu::draw(frame, &ui))?;
        }
    }
    if state.history.is_session_recording {
        state.history.stop_and_save_session();
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
    if state.selected_pid.is_none() && !display_pids.is_empty() {
        state.selected_pid = display_pids.first().copied();
    }
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

#[cfg(test)]
mod tests {

    use super::*;
    use crate::data::cpu::ProcessInfo;

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
    fn collection_profile_follows_visible_view() {
        let mut state = State::default();
        assert_eq!(visible_profile(&state), CollectionProfile::Dashboard);
        assert_eq!(
            collection_plan(&state),
            CollectionPlan::new(CollectionProfile::Dashboard, false)
        );

        state.fullscreen = true;
        for (pane, profile) in [
            (Pane::Cpu, CollectionProfile::Cpu),
            (Pane::Io, CollectionProfile::Io),
            (Pane::Net, CollectionProfile::Net),
            (Pane::Mem, CollectionProfile::Mem),
            (Pane::Disks, CollectionProfile::Disks),
            (Pane::Gpu, CollectionProfile::Gpu),
            (Pane::History, CollectionProfile::History),
        ] {
            state.pane = pane;
            assert_eq!(visible_profile(&state), profile);
            assert_eq!(collection_plan(&state), CollectionPlan::new(profile, false));
        }
    }

    #[test]
    fn session_recording_updates_collection_plan() {
        let mut state = State::default();
        state.history.is_session_recording = true;
        assert_eq!(
            collection_plan(&state),
            CollectionPlan::new(CollectionProfile::Dashboard, true)
        );

        state.fullscreen = true;
        state.pane = Pane::Cpu;
        assert_eq!(
            collection_plan(&state),
            CollectionPlan::new(CollectionProfile::Cpu, true)
        );
        assert_eq!(visible_profile(&state), CollectionProfile::Cpu);
    }
}

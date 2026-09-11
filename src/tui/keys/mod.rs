//! Key dispatch: the modal handlers get first refusal, then the normal ones.

mod groups;
mod modal;
mod normal;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::theme::Theme;

use super::cpu::Pane;
use super::{Lang, State, PAGE_STEP};

use modal::{
    handle_help_key, handle_kill_key, handle_record_modal_key, handle_searching_key,
    handle_sessions_modal_key, handle_tracing_key,
};
use normal::{handle_menu_key, handle_normal_key};

pub(crate) fn handle_key(
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
    if state.pane == Pane::History && state.history.sessions_modal {
        handle_sessions_modal_key(state, code);
        return false;
    }
    if state.pane == Pane::History && state.history.record_modal {
        handle_record_modal_key(state, code);
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

pub(super) fn select_menu_pane(state: &mut State, pane: Pane) {
    state.pane = pane;
    state.fullscreen = true;
    state.show_menu = false;
}

/// Expand `target` fullscreen, or drop back to the dashboard when the same
/// pane's number is pressed again.
pub(super) fn focus_pane(state: &mut State, target: Pane) {
    if state.fullscreen && state.pane == target {
        state.fullscreen = false;
    } else {
        state.pane = target;
        state.fullscreen = true;
    }
}

pub(super) fn toggle_lang(state: &mut State) -> bool {
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

pub(super) fn toggle_theme(state: &mut State, system_theme: Option<Theme>) -> bool {
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

pub(super) fn start_trace(state: &mut State) -> bool {
    if let Some(pid) = state.selected_pid {
        state.status_msg = None;
        state.tracing = true;
        state.trace_start_pid = Some(pid);
    }
    false
}

pub(super) fn toggle_core_filter(state: &mut State) -> bool {
    if state.pane == Pane::Cpu {
        state.core_filter = match state.core_filter {
            Some(c) if c == state.core_focus => None,
            _ => Some(state.core_focus),
        };
    }
    false
}

pub(super) fn handle_nav_key(state: &mut State, display_pids: &[u32], code: KeyCode) -> bool {
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

pub(super) fn move_core(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Left => state.core_focus = state.core_focus.saturating_sub(1),
        KeyCode::Right => state.core_focus += 1,
        KeyCode::Up => state.core_focus = state.core_focus.saturating_sub(2),
        KeyCode::Down => state.core_focus += 2,
        _ => {}
    }
}

pub(super) fn move_selection(state: &mut State, display_pids: &[u32], delta: i32) {
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

pub(super) fn send_signal(state: &mut State, sig: i32) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

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
        handle_key(&mut s, &[], KeyCode::Char('m'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('?'), KeyModifiers::empty(), None);
        assert!(s.help);
        assert!(!s.show_menu);
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
}

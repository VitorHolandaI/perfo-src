//! Keys outside any modal, grouped by what the key means.

use crossterm::event::KeyCode;

use crate::theme::Theme;

use super::super::clipboard::{copy_to_clipboard, get_process_full_cmd};
use super::super::cpu::{Pane, SortKey};
use super::super::{Lang, State};
use super::{
    focus_pane, handle_nav_key, move_core, select_menu_pane, start_trace, toggle_core_filter,
    toggle_lang, toggle_theme,
};

/// `1`-`7` jump straight to a fullscreen pane.
pub(super) fn pane_for_digit(digit: char) -> Option<Pane> {
    match digit {
        '1' => Some(Pane::Cpu),
        '2' => Some(Pane::Io),
        '3' => Some(Pane::Net),
        '4' => Some(Pane::Mem),
        '5' => Some(Pane::Disks),
        '6' => Some(Pane::Gpu),
        '7' => Some(Pane::History),
        _ => None,
    }
}

/// Keys that only act on the history pane. `None` means this handler did not
/// claim the key, so the caller keeps looking.
pub(super) fn handle_history_scroll_key(state: &mut State, code: KeyCode) -> Option<bool> {
    match code {
        KeyCode::Char('<') | KeyCode::Char(',') => {
            if state.pane == Pane::History {
                state.history.step(-1);
            }
            Some(false)
        }
        KeyCode::Char('>') | KeyCode::Char('.') => {
            if state.pane == Pane::History {
                state.history.step(1);
            }
            Some(false)
        }
        KeyCode::Char('[') | KeyCode::Char('{') => {
            if state.pane == Pane::History {
                state.history.jump(-1);
            }
            Some(false)
        }
        KeyCode::Char(']') | KeyCode::Char('}') => {
            if state.pane == Pane::History {
                state.history.jump(1);
            }
            Some(false)
        }
        KeyCode::Char('0') => {
            if state.pane == Pane::History {
                state.history.jump_to_live();
            }
            Some(false)
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if state.pane == Pane::History {
                state.history.export();
            }
            Some(false)
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if state.pane == Pane::History {
                state.history.toggle_session_recording();
            }
            Some(false)
        }
        _ => None,
    }
}

/// Keys that flip a single flag and never quit.
pub(super) fn handle_toggle_key(state: &mut State, code: KeyCode) -> Option<bool> {
    match code {
        KeyCode::Char('p') | KeyCode::Char('P') => {
            state.sort = SortKey::Cpu;
            Some(false)
        }
        KeyCode::Char('M') => {
            state.sort = SortKey::Mem;
            Some(false)
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            state.invert = !state.invert;
            Some(false)
        }
        KeyCode::Char('c') => {
            state.full_cmd = !state.full_cmd;
            Some(false)
        }
        KeyCode::Char('t') => {
            state.tree = !state.tree;
            Some(false)
        }
        KeyCode::Char('H') => {
            state.show_threads = !state.show_threads;
            Some(false)
        }
        KeyCode::Char('K') => {
            state.show_kernel = !state.show_kernel;
            Some(false)
        }
        KeyCode::Char('m') => {
            state.show_menu = !state.show_menu;
            Some(false)
        }
        KeyCode::Char('/') => {
            state.searching = true;
            Some(false)
        }
        _ => None,
    }
}

/// Keys whose meaning depends on which pane is focused: on history they scrub
/// or switch the recording view, elsewhere they keep their global meaning.
pub(super) fn handle_pane_aware_key(state: &mut State, code: KeyCode) -> Option<bool> {
    Some(match code {
        KeyCode::Tab | KeyCode::BackTab => {
            if state.pane == Pane::Cpu {
                state.cores_focused = !state.cores_focused;
            } else if state.pane == Pane::History {
                state.history.metric = state.history.metric.next();
            }
            false
        }
        KeyCode::Char('L') => {
            if state.pane == Pane::History {
                state.history.jump_to_live();
                false
            } else {
                toggle_lang(state)
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if state.pane == Pane::History {
                state.history.open_sessions_modal();
                false
            } else {
                start_trace(state)
            }
        }
        KeyCode::Char('z') | KeyCode::Char('Z') => {
            if state.pane == Pane::History {
                state.history.span = state.history.span.next();
            } else {
                state.paused = !state.paused;
            }
            false
        }
        KeyCode::Left => {
            if state.pane == Pane::History {
                state.history.step(-1);
                false
            } else if state.pane == Pane::Cpu && state.cores_focused {
                move_core(state, code);
                false
            } else {
                state.cmd_scroll = state.cmd_scroll.saturating_sub(10);
                false
            }
        }
        KeyCode::Right => {
            if state.pane == Pane::History {
                state.history.step(1);
                false
            } else if state.pane == Pane::Cpu && state.cores_focused {
                move_core(state, code);
                false
            } else {
                state.cmd_scroll = state.cmd_scroll.saturating_add(10);
                false
            }
        }
        KeyCode::Char(' ') => {
            if state.pane == Pane::History {
                state.history.toggle_playback();
            } else if state.cores_focused {
                toggle_core_filter(state);
            }
            false
        }
        _ => return None,
    })
}

pub(super) fn handle_normal_key(
    state: &mut State,
    display_pids: &[u32],
    code: KeyCode,
    system_theme: Option<Theme>,
) -> bool {
    if !state.fullscreen {
        return handle_dashboard_key(state, code, system_theme);
    }
    if let KeyCode::Char(digit @ '1'..='7') = code {
        if let Some(pane) = pane_for_digit(digit) {
            focus_pane(state, pane);
            return false;
        }
    }
    if let Some(quit) = handle_history_scroll_key(state, code) {
        return quit;
    }
    if let Some(quit) = handle_toggle_key(state, code) {
        return quit;
    }
    if let Some(quit) = handle_pane_aware_key(state, code) {
        return quit;
    }
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
        KeyCode::Char('k') => {
            if state.selected_pid.is_some() {
                state.status_msg = None;
                state.kill_prompt = true;
            }
            false
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let target_pid = state.selected_pid.or_else(|| display_pids.first().copied());
            if let Some(pid) = target_pid {
                state.selected_pid = Some(pid);
                let cmd = get_process_full_cmd(pid);
                copy_to_clipboard(&cmd);
                state.status_msg = Some(match state.lang {
                    Lang::Pt => format!("Comando do PID {pid} copiado"),
                    Lang::En => format!("Copied command of PID {pid}"),
                });
            }
            false
        }
        KeyCode::Char('?') | KeyCode::F(1) | KeyCode::Char('h') => {
            state.help = true;
            false
        }
        KeyCode::Char('C') => toggle_theme(state, system_theme),
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End => handle_nav_key(state, display_pids, code),
        KeyCode::Enter => {
            if state.cores_focused {
                toggle_core_filter(state);
            } else {
                let target_pid = state.selected_pid.or_else(|| display_pids.first().copied());
                if let Some(pid) = target_pid {
                    state.selected_pid = Some(pid);
                    let cmd = get_process_full_cmd(pid);
                    copy_to_clipboard(&cmd);
                    state.status_msg = Some(match state.lang {
                        Lang::Pt => format!("Comando do PID {pid} copiado"),
                        Lang::En => format!("Copied command of PID {pid}"),
                    });
                }
            }
            false
        }
        _ => false,
    }
}

pub(super) fn handle_dashboard_key(
    state: &mut State,
    code: KeyCode,
    system_theme: Option<Theme>,
) -> bool {
    if matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc) {
        return true;
    }
    match code {
        KeyCode::Char('1') => focus_pane(state, Pane::Cpu),
        KeyCode::Char('2') => focus_pane(state, Pane::Io),
        KeyCode::Char('3') => focus_pane(state, Pane::Net),
        KeyCode::Char('4') => focus_pane(state, Pane::Mem),
        KeyCode::Char('5') => focus_pane(state, Pane::Disks),
        KeyCode::Char('6') => focus_pane(state, Pane::Gpu),
        KeyCode::Char('7') => focus_pane(state, Pane::History),
        KeyCode::Char('?') | KeyCode::F(1) | KeyCode::Char('h') => state.help = true,
        KeyCode::Char('m') => state.show_menu = !state.show_menu,
        KeyCode::Char('z') | KeyCode::Char('Z') => state.paused = !state.paused,
        KeyCode::Char('C') => {
            toggle_theme(state, system_theme);
        }
        KeyCode::Char('L') => {
            toggle_lang(state);
        }
        _ => {}
    }
    false
}

pub(super) fn handle_menu_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Char('m') | KeyCode::Esc => state.show_menu = false,
        KeyCode::Char('1') => select_menu_pane(state, Pane::Cpu),
        KeyCode::Char('2') => select_menu_pane(state, Pane::Io),
        KeyCode::Char('3') => select_menu_pane(state, Pane::Net),
        KeyCode::Char('4') => select_menu_pane(state, Pane::Mem),
        KeyCode::Char('5') => select_menu_pane(state, Pane::Disks),
        KeyCode::Char('6') => select_menu_pane(state, Pane::Gpu),
        KeyCode::Char('7') => select_menu_pane(state, Pane::History),
        KeyCode::Char('?')
        | KeyCode::Char('h')
        | KeyCode::Char('H')
        | KeyCode::F(1)
        | KeyCode::Char('8') => {
            state.show_menu = false;
            state.help = true;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {

    use super::super::handle_key;
    use super::super::move_selection;
    use super::*;

    use crossterm::event::{KeyCode, KeyModifiers};
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
        let mut s = State {
            fullscreen: true,
            ..State::default()
        };
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
    fn pane_for_digit_maps_every_fullscreen_key() {
        let expected = [
            ('1', Pane::Cpu),
            ('2', Pane::Io),
            ('3', Pane::Net),
            ('4', Pane::Mem),
            ('5', Pane::Disks),
            ('6', Pane::Gpu),
            ('7', Pane::History),
        ];
        for (digit, pane) in expected {
            assert_eq!(pane_for_digit(digit), Some(pane), "digit {digit}");
        }
        for digit in ['0', '8', '9', 'a'] {
            assert_eq!(pane_for_digit(digit), None, "digit {digit}");
        }
    }

    #[test]
    fn key_group_handlers_only_claim_their_own_keys() {
        let mut state = State::default();

        // A key no group owns falls through every handler.
        assert_eq!(
            handle_history_scroll_key(&mut state, KeyCode::Char('q')),
            None
        );
        assert_eq!(handle_toggle_key(&mut state, KeyCode::Char('q')), None);
        assert_eq!(handle_pane_aware_key(&mut state, KeyCode::Char('q')), None);

        // Each group claims a key of its own and never asks to quit.
        assert_eq!(
            handle_history_scroll_key(&mut state, KeyCode::Char('0')),
            Some(false)
        );
        assert_eq!(
            handle_toggle_key(&mut state, KeyCode::Char('t')),
            Some(false)
        );
        assert_eq!(
            handle_pane_aware_key(&mut state, KeyCode::Char(' ')),
            Some(false)
        );
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
        handle_key(&mut s, &[], KeyCode::Char('7'), KeyModifiers::empty(), None);
        assert_eq!(s.pane, Pane::History);
        assert!(s.fullscreen);
        handle_key(&mut s, &[], KeyCode::Char('7'), KeyModifiers::empty(), None);
        assert!(!s.fullscreen);
    }

    #[test]
    fn dashboard_ignores_detail_only_shortcuts() {
        let mut state = State::default();
        handle_key(&mut state, &[], KeyCode::Tab, KeyModifiers::empty(), None);
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('/'),
            KeyModifiers::empty(),
            None,
        );
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('s'),
            KeyModifiers::empty(),
            None,
        );
        assert!(!state.cores_focused);
        assert!(!state.searching);
        assert!(!state.tracing);
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
    fn esc_clears_core_filter_then_quits() {
        let mut s = State {
            core_filter: Some(2),
            fullscreen: true,
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
    fn tab_switches_cpu_subfocus_without_changing_panel() {
        let mut s = State {
            fullscreen: true,
            ..State::default()
        };
        assert!(!s.cores_focused);
        handle_key(&mut s, &[], KeyCode::Tab, KeyModifiers::empty(), None);
        assert!(s.cores_focused);
        assert_eq!(s.pane, Pane::Cpu);
        handle_key(&mut s, &[], KeyCode::Tab, KeyModifiers::empty(), None);
        assert!(!s.cores_focused);
    }

    #[test]
    fn horizontal_scroll_with_procs_focused() {
        let mut s = State {
            cores_focused: false,
            fullscreen: true,
            ..State::default()
        };
        assert_eq!(s.cmd_scroll, 0);
        handle_key(&mut s, &[], KeyCode::Right, KeyModifiers::empty(), None);
        assert_eq!(s.cmd_scroll, 10);
        handle_key(&mut s, &[], KeyCode::Right, KeyModifiers::empty(), None);
        assert_eq!(s.cmd_scroll, 20);
        handle_key(&mut s, &[], KeyCode::Left, KeyModifiers::empty(), None);
        assert_eq!(s.cmd_scroll, 10);
        handle_key(&mut s, &[], KeyCode::Left, KeyModifiers::empty(), None);
        assert_eq!(s.cmd_scroll, 0);
        handle_key(&mut s, &[], KeyCode::Left, KeyModifiers::empty(), None);
        assert_eq!(s.cmd_scroll, 0);
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
}

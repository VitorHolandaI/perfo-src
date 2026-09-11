//! Keys outside any modal, grouped by what the key means.

use crossterm::event::KeyCode;

use super::groups::{
    handle_history_scroll_key, handle_pane_aware_key, handle_toggle_key, pane_for_digit,
};

use crate::theme::Theme;

use super::super::clipboard::{copy_to_clipboard, get_process_full_cmd};
use super::super::cpu::Pane;
use super::super::{Lang, State};
use super::{
    focus_pane, handle_nav_key, select_menu_pane, toggle_core_filter, toggle_lang, toggle_theme,
};

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

    use super::*;
    use crate::tui::cpu::SortKey;

    use crossterm::event::{KeyCode, KeyModifiers};

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
}

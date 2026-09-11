//! The fullscreen keys, grouped by what the key means. Each group returns
//! `None` for a key it does not own so the caller keeps looking.

use crossterm::event::KeyCode;

use super::super::cpu::{Pane, SortKey};
use super::super::State;
use super::{move_core, start_trace, toggle_core_filter, toggle_lang};

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

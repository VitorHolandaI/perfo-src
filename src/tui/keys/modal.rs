//! Keys consumed by an open modal: the session list, the recording picker,
//! help, the trace view, the search field and the kill prompt.

use crossterm::event::KeyCode;

/// The recording picker lists six subsystems, then the two buttons.
const LAST_SUBSYSTEM_IDX: usize = 5;
const START_BUTTON_IDX: usize = 6;
const CANCEL_BUTTON_IDX: usize = 7;

use super::super::{State, HELP_LAST_PAGE};
use super::{send_signal, toggle_lang};

pub(super) fn handle_sessions_modal_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Esc
        | KeyCode::Char('q')
        | KeyCode::Char('Q')
        | KeyCode::Char('s')
        | KeyCode::Char('S') => {
            state.history.close_sessions_modal();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.history.modal_prev();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.history.modal_next();
        }
        KeyCode::Enter => {
            state.history.modal_load_selected();
            state.history.close_sessions_modal();
        }
        KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
            state.history.modal_delete_selected();
        }
        _ => {}
    }
}

pub(super) fn handle_record_modal_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            state.history.close_record_modal();
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            state.history.start_session_recording();
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
            state.history.record_modal_prev();
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            state.history.record_modal_next();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if state.history.record_modal_idx == CANCEL_BUTTON_IDX {
                state.history.record_modal_idx = START_BUTTON_IDX;
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if state.history.record_modal_idx == START_BUTTON_IDX {
                state.history.record_modal_idx = CANCEL_BUTTON_IDX;
            }
        }
        KeyCode::Char('1') => state.history.toggle_record_mask_item(0),
        KeyCode::Char('2') => state.history.toggle_record_mask_item(1),
        KeyCode::Char('3') => state.history.toggle_record_mask_item(2),
        KeyCode::Char('4') => state.history.toggle_record_mask_item(3),
        KeyCode::Char('5') => state.history.toggle_record_mask_item(4),
        KeyCode::Char('6') => state.history.toggle_record_mask_item(5),
        KeyCode::Char(' ') => match state.history.record_modal_idx {
            0..=LAST_SUBSYSTEM_IDX => state
                .history
                .toggle_record_mask_item(state.history.record_modal_idx),
            6 => state.history.start_session_recording(),
            7 => state.history.close_record_modal(),
            _ => {}
        },
        KeyCode::Enter => match state.history.record_modal_idx {
            0..=LAST_SUBSYSTEM_IDX => state
                .history
                .toggle_record_mask_item(state.history.record_modal_idx),
            6 => state.history.start_session_recording(),
            7 => state.history.close_record_modal(),
            _ => {}
        },
        _ => {}
    }
}

pub(super) fn handle_help_key(state: &mut State, code: KeyCode) {
    match code {
        KeyCode::Right
        | KeyCode::Char('l')
        | KeyCode::PageDown
        | KeyCode::Char('n')
        | KeyCode::Char('j') => {
            state.help_page = (state.help_page + 1).min(HELP_LAST_PAGE);
        }
        KeyCode::Left | KeyCode::PageUp | KeyCode::Char('p') | KeyCode::Char('k') => {
            state.help_page = state.help_page.saturating_sub(1);
        }
        KeyCode::Char('?')
        | KeyCode::F(1)
        | KeyCode::Char('h')
        | KeyCode::Char('q')
        | KeyCode::Char('Q')
        | KeyCode::Esc => {
            state.help = false;
        }
        KeyCode::Char('L') => {
            toggle_lang(state);
        }
        _ => {}
    }
}

pub(super) fn handle_tracing_key(state: &mut State, code: KeyCode) {
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

pub(super) fn handle_searching_key(state: &mut State, code: KeyCode) {
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

pub(super) fn handle_kill_key(state: &mut State, code: KeyCode) {
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

#[cfg(test)]
mod tests {

    use super::super::handle_key;
    use super::*;
    use crate::tui::cpu::Pane;

    use crossterm::event::{KeyCode, KeyModifiers};
    #[test]
    fn help_navigates_pages_and_closes() {
        let mut s = State::default();
        handle_key(&mut s, &[], KeyCode::Char('h'), KeyModifiers::empty(), None);
        assert!(s.help);
        handle_key(&mut s, &[], KeyCode::Right, KeyModifiers::empty(), None);
        assert_eq!(s.help_page, 1);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        handle_key(&mut s, &[], KeyCode::Char('n'), KeyModifiers::empty(), None);
        assert_eq!(s.help_page, HELP_LAST_PAGE);
        handle_key(&mut s, &[], KeyCode::Left, KeyModifiers::empty(), None);
        assert_eq!(s.help_page, 4);
        handle_key(&mut s, &[], KeyCode::Char('h'), KeyModifiers::empty(), None);
        assert!(!s.help);
    }

    #[test]
    fn search_typing_and_escape() {
        let mut s = State {
            fullscreen: true,
            ..State::default()
        };
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
    fn kill_prompt_requires_selection() {
        let mut s = State {
            fullscreen: true,
            ..State::default()
        };
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

    #[test]
    fn history_r_opens_record_modal_and_handles_keys() {
        let mut state = State {
            fullscreen: true,
            pane: Pane::History,
            ..Default::default()
        };
        assert!(!state.history.record_modal);

        // Press 'r' to open modal
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('r'),
            KeyModifiers::empty(),
            None,
        );
        assert!(state.history.record_modal);
        assert_eq!(state.history.record_modal_idx, 0);

        // Direct toggle '2' toggles MEM (idx 1)
        assert!(state.history.recording_mask.mem);
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('2'),
            KeyModifiers::empty(),
            None,
        );
        assert!(!state.history.recording_mask.mem);

        // Press Down to move to item 1
        handle_key(&mut state, &[], KeyCode::Down, KeyModifiers::empty(), None);
        assert_eq!(state.history.record_modal_idx, 1);

        // Press Space to toggle MEM back on
        handle_key(
            &mut state,
            &[],
            KeyCode::Char(' '),
            KeyModifiers::empty(),
            None,
        );
        assert!(state.history.recording_mask.mem);

        // Press 'r' in modal to start recording
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('r'),
            KeyModifiers::empty(),
            None,
        );
        assert!(!state.history.record_modal);
        assert!(state.history.is_session_recording);

        // Press 'r' again while recording stops and saves
        handle_key(
            &mut state,
            &[],
            KeyCode::Char('r'),
            KeyModifiers::empty(),
            None,
        );
        assert!(!state.history.is_session_recording);
    }
}

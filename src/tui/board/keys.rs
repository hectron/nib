use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::{Board, Input, MenuKey, Overlay, TextEdit};
use crate::domain::keymap::{KeyChord, State as InputState};

pub(crate) fn inputs(board: &Board, key: KeyEvent) -> Vec<Input> {
    if let Some(overlay) = board.overlay() {
        let menu = menu_key(key).or((overlay == Overlay::Help).then_some(MenuKey::Cancel));
        return menu.map(Input::Menu).into_iter().collect();
    }
    let state = board.mode();
    if let Some(actions) = chord(key).and_then(|chord| {
        board
            .config()
            .tui
            .keymap
            .actions(state, &chord)
            .map(<[_]>::to_vec)
    }) {
        return actions.into_iter().map(Input::Action).collect();
    }
    if state == InputState::Browse {
        return Vec::new();
    }
    text_edit(key).map(Input::Text).into_iter().collect()
}

pub(crate) fn paste(board: &Board, text: &str) -> Vec<Input> {
    if board.overlay().is_some() || board.mode() == InputState::Browse {
        return Vec::new();
    }
    text.chars()
        .filter_map(|character| match character {
            '\n' | '\r' | '\t' => Some(' '),
            character if character.is_control() => None,
            character => Some(character),
        })
        .map(|character| Input::Text(TextEdit::Insert(character)))
        .collect()
}

pub(crate) fn chord(key: KeyEvent) -> Option<KeyChord> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let mut text = String::new();
    if ctrl {
        text.push_str("ctrl-");
    }
    if alt {
        text.push_str("alt-");
    }
    let named = match key.code {
        KeyCode::Char(' ') => "space",
        KeyCode::Char(character) => {
            if (ctrl || alt) && character.is_ascii_alphabetic() {
                if shift || character.is_ascii_uppercase() {
                    text.push_str("shift-");
                }
                text.push(character.to_ascii_lowercase());
            } else {
                text.push(character);
            }
            return text.parse().ok();
        }
        KeyCode::BackTab => {
            text.push_str("shift-tab");
            return text.parse().ok();
        }
        KeyCode::Enter => "enter",
        KeyCode::Esc => "esc",
        KeyCode::Tab => "tab",
        KeyCode::Up => "up",
        KeyCode::Down => "down",
        KeyCode::Left => "left",
        KeyCode::Right => "right",
        KeyCode::Home => "home",
        KeyCode::End => "end",
        KeyCode::PageUp => "page-up",
        KeyCode::PageDown => "page-down",
        KeyCode::Backspace => "backspace",
        KeyCode::Delete => "delete",
        KeyCode::Insert => "insert",
        KeyCode::F(number) => {
            if shift {
                text.push_str("shift-");
            }
            text.push('f');
            text.push_str(&number.to_string());
            return text.parse().ok();
        }
        _ => return None,
    };
    if shift && key.code != KeyCode::Char(' ') {
        text.push_str("shift-");
    }
    text.push_str(named);
    text.parse().ok()
}

fn text_edit(key: KeyEvent) -> Option<TextEdit> {
    let modifiers = key.modifiers - KeyModifiers::SHIFT;
    let ctrl = modifiers == KeyModifiers::CONTROL;
    let alt = modifiers == KeyModifiers::ALT;
    match key.code {
        KeyCode::Char(character) if modifiers.is_empty() => Some(TextEdit::Insert(character)),
        KeyCode::Backspace if modifiers.is_empty() => Some(TextEdit::Backspace),
        KeyCode::Char('h') if ctrl => Some(TextEdit::Backspace),
        KeyCode::Backspace if alt => Some(TextEdit::KillWord),
        KeyCode::Char('w') if ctrl => Some(TextEdit::KillWord),
        KeyCode::Delete if modifiers.is_empty() => Some(TextEdit::DeleteForward),
        KeyCode::Left if modifiers.is_empty() => Some(TextEdit::Left),
        KeyCode::Char('b') if ctrl => Some(TextEdit::Left),
        KeyCode::Right if modifiers.is_empty() => Some(TextEdit::Right),
        KeyCode::Char('f') if ctrl => Some(TextEdit::Right),
        _ => None,
    }
}

fn menu_key(key: KeyEvent) -> Option<MenuKey> {
    let modifiers = key.modifiers - KeyModifiers::SHIFT;
    let ctrl = modifiers == KeyModifiers::CONTROL;
    match key.code {
        KeyCode::Enter => Some(MenuKey::Accept),
        KeyCode::Esc => Some(MenuKey::Cancel),
        KeyCode::Char('c' | 'g') if ctrl => Some(MenuKey::Cancel),
        KeyCode::Up => Some(MenuKey::Up),
        KeyCode::Char('p') if ctrl => Some(MenuKey::Up),
        KeyCode::Down => Some(MenuKey::Down),
        KeyCode::Char('n') if ctrl => Some(MenuKey::Down),
        KeyCode::Backspace => Some(MenuKey::Backspace),
        KeyCode::Char(character) if modifiers.is_empty() => Some(MenuKey::Char(character)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Config;
    use crate::domain::keymap::Action;
    use crate::tui::board::state::tests::board_with;
    use crate::tui::state::tests::{data, now};

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn name(code: KeyCode, modifiers: KeyModifiers) -> Option<String> {
        chord(key(code, modifiers)).map(|chord| chord.to_string())
    }

    #[test]
    fn terminal_keys_become_canonical_chords() {
        let cases = [
            (KeyCode::Char('x'), NONE, "x"),
            (KeyCode::Char('G'), SHIFT, "shift-g"),
            (KeyCode::Char('?'), SHIFT, "?"),
            (KeyCode::Char('o'), CTRL, "ctrl-o"),
            (KeyCode::Char('G'), CTRL | SHIFT, "ctrl-shift-g"),
            (KeyCode::Char('a'), ALT, "alt-a"),
            (KeyCode::Char(' '), NONE, "space"),
            (KeyCode::Char(' '), CTRL, "ctrl-space"),
            (KeyCode::Char('-'), NONE, "-"),
            (KeyCode::BackTab, SHIFT, "shift-tab"),
            (KeyCode::Enter, ALT, "alt-enter"),
            (KeyCode::PageDown, NONE, "page-down"),
            (KeyCode::F(1), NONE, "f1"),
            (KeyCode::Delete, NONE, "delete"),
        ];
        for (code, modifiers, expected) in cases {
            assert_eq!(
                name(code, modifiers).as_deref(),
                Some(expected),
                "{code:?} {modifiers:?}"
            );
        }
        assert_eq!(name(KeyCode::Null, NONE), None);
    }

    #[test]
    fn browse_keys_run_the_configured_action_sequence() {
        let mut config = Config::default();
        config.tui.keymap.bind(
            InputState::Browse,
            "alt-x".parse().unwrap(),
            vec![Action::Done, Action::Down],
        );
        let board = board_with(data(), config);
        assert_eq!(
            inputs(&board, key(KeyCode::Char('x'), ALT)),
            [Input::Action(Action::Done), Input::Action(Action::Down)]
        );
        assert_eq!(
            inputs(&board, key(KeyCode::Char('/'), NONE)),
            [Input::Action(Action::Filter)]
        );
        assert_eq!(inputs(&board, key(KeyCode::Char('z'), NONE)), []);
    }

    #[test]
    fn unbound_keys_edit_text_only_while_typing() {
        let mut board = board_with(data(), Config::default());
        board.update(Input::Action(Action::Filter));
        assert_eq!(
            inputs(&board, key(KeyCode::Char('x'), NONE)),
            [Input::Text(TextEdit::Insert('x'))]
        );
        assert_eq!(
            inputs(&board, key(KeyCode::Char('w'), CTRL)),
            [Input::Text(TextEdit::KillWord)]
        );
        assert_eq!(
            inputs(&board, key(KeyCode::Esc, NONE)),
            [Input::Action(Action::Cancel)]
        );
        assert_eq!(inputs(&board, key(KeyCode::Char('x'), ALT)), []);
    }

    #[test]
    fn pasted_text_is_typed_only_while_typing() {
        let mut board = board_with(data(), Config::default());
        assert_eq!(paste(&board, "xr1e"), []);
        board.update(Input::Action(Action::Capture));
        assert_eq!(
            paste(&board, "a\nb\u{7}"),
            [
                Input::Text(TextEdit::Insert('a')),
                Input::Text(TextEdit::Insert(' ')),
                Input::Text(TextEdit::Insert('b')),
            ]
        );
    }

    #[test]
    fn menus_use_fixed_keys_and_any_key_closes_help() {
        let mut board = board_with(data(), Config::default());
        board.update(Input::Action(Action::ActMenu));
        assert_eq!(
            inputs(&board, key(KeyCode::Char('x'), NONE)),
            [Input::Menu(MenuKey::Char('x'))]
        );
        assert_eq!(
            inputs(&board, key(KeyCode::Char('g'), CTRL)),
            [Input::Menu(MenuKey::Cancel)]
        );
        assert_eq!(inputs(&board, key(KeyCode::F(2), NONE)), []);

        let (mut help, _) = crate::tui::board::state::Board::new(
            data(),
            Vec::new(),
            Config::default(),
            now(),
            None,
        );
        help.update(Input::Action(Action::Help));
        assert_eq!(
            inputs(&help, key(KeyCode::F(2), NONE)),
            [Input::Menu(MenuKey::Cancel)]
        );
    }
}

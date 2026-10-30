use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::{Action, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    Prompt,
    Confirm,
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

const BINDINGS: &[(Scope, KeyCode, KeyModifiers, Action)] = &[
    (Scope::Any, KeyCode::Char('c'), CTRL, Action::Quit),
    (Scope::Any, KeyCode::Esc, NONE, Action::Cancel),
    (Scope::Any, KeyCode::Char('g'), CTRL, Action::Cancel),
    (Scope::Any, KeyCode::Enter, NONE, Action::Accept),
    (Scope::Any, KeyCode::Char('n'), CTRL, Action::Down),
    (Scope::Any, KeyCode::Char('p'), CTRL, Action::Up),
    (Scope::Any, KeyCode::Down, NONE, Action::Down),
    (Scope::Any, KeyCode::Up, NONE, Action::Up),
    (Scope::Any, KeyCode::Tab, NONE, Action::ToggleDown),
    (Scope::Any, KeyCode::BackTab, NONE, Action::ToggleUp),
    (Scope::Prompt, KeyCode::Char('o'), CTRL, Action::OpenMenu),
    (Scope::Prompt, KeyCode::Backspace, NONE, Action::Backspace),
    (Scope::Prompt, KeyCode::Char('h'), CTRL, Action::Backspace),
    (Scope::Prompt, KeyCode::Delete, NONE, Action::DeleteForward),
    (
        Scope::Prompt,
        KeyCode::Char('d'),
        CTRL,
        Action::DeleteForward,
    ),
    (Scope::Prompt, KeyCode::Left, NONE, Action::CaretLeft),
    (Scope::Prompt, KeyCode::Char('b'), CTRL, Action::CaretLeft),
    (Scope::Prompt, KeyCode::Right, NONE, Action::CaretRight),
    (Scope::Prompt, KeyCode::Char('f'), CTRL, Action::CaretRight),
    (Scope::Prompt, KeyCode::Home, NONE, Action::CaretStart),
    (Scope::Prompt, KeyCode::Char('a'), CTRL, Action::CaretStart),
    (Scope::Prompt, KeyCode::End, NONE, Action::CaretEnd),
    (Scope::Prompt, KeyCode::Char('e'), CTRL, Action::CaretEnd),
    (Scope::Prompt, KeyCode::Char('u'), CTRL, Action::KillToStart),
    (Scope::Prompt, KeyCode::Char('k'), CTRL, Action::KillToEnd),
    (Scope::Prompt, KeyCode::Char('w'), CTRL, Action::KillWord),
    (Scope::Prompt, KeyCode::Backspace, ALT, Action::KillWord),
    (Scope::Confirm, KeyCode::Backspace, NONE, Action::Backspace),
    (Scope::Confirm, KeyCode::Char('h'), CTRL, Action::Backspace),
];

pub(crate) fn action(mode: Mode, key: KeyEvent) -> Option<Action> {
    let prompt = mode == Mode::Filter;
    let modifiers = key.modifiers - KeyModifiers::SHIFT;
    let bound = BINDINGS
        .iter()
        .find(|(scope, code, wanted, _)| {
            let active = match scope {
                Scope::Any => true,
                Scope::Prompt => prompt,
                Scope::Confirm => mode == Mode::ConfirmDelete,
            };
            active && *code == key.code && *wanted == modifiers
        })
        .map(|(.., action)| *action);
    bound.or(match key.code {
        KeyCode::Char(character) if modifiers.is_empty() => Some(if prompt {
            Action::Insert(character)
        } else {
            Action::Hotkey(character)
        }),
        _ => None,
    })
}

pub(crate) fn label(action: Action) -> Option<String> {
    BINDINGS
        .iter()
        .find(|(.., bound)| *bound == action)
        .map(|(_, code, modifiers, _)| {
            let key = match code {
                KeyCode::Char(character) => character.to_string(),
                KeyCode::Esc => "esc".into(),
                KeyCode::Enter => "enter".into(),
                KeyCode::Tab => "tab".into(),
                KeyCode::BackTab => "shift-tab".into(),
                KeyCode::Up => "up".into(),
                KeyCode::Down => "down".into(),
                other => other.to_string().to_lowercase(),
            };
            if modifiers.contains(CTRL) {
                format!("ctrl-{key}")
            } else if modifiers.contains(ALT) {
                format!("alt-{key}")
            } else {
                key
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn typing_inserts_in_the_prompt_and_picks_in_menus() {
        assert_eq!(
            action(Mode::Filter, key(KeyCode::Char('x'), NONE)),
            Some(Action::Insert('x'))
        );
        assert_eq!(
            action(Mode::Filter, key(KeyCode::Char('X'), KeyModifiers::SHIFT)),
            Some(Action::Insert('X'))
        );
        assert_eq!(
            action(Mode::Act, key(KeyCode::Char('x'), NONE)),
            Some(Action::Hotkey('x'))
        );
        assert_eq!(action(Mode::Filter, key(KeyCode::Char('x'), ALT)), None);
    }

    #[test]
    fn readline_and_fzf_defaults_are_bound() {
        let cases = [
            (KeyCode::Char('n'), CTRL, Action::Down),
            (KeyCode::Char('p'), CTRL, Action::Up),
            (KeyCode::Tab, NONE, Action::ToggleDown),
            (KeyCode::BackTab, KeyModifiers::SHIFT, Action::ToggleUp),
            (KeyCode::Char('o'), CTRL, Action::OpenMenu),
            (KeyCode::Char('a'), CTRL, Action::CaretStart),
            (KeyCode::Char('e'), CTRL, Action::CaretEnd),
            (KeyCode::Char('w'), CTRL, Action::KillWord),
            (KeyCode::Char('u'), CTRL, Action::KillToStart),
            (KeyCode::Char('k'), CTRL, Action::KillToEnd),
            (KeyCode::Esc, NONE, Action::Cancel),
            (KeyCode::Char('g'), CTRL, Action::Cancel),
            (KeyCode::Char('c'), CTRL, Action::Quit),
            (KeyCode::Enter, NONE, Action::Accept),
        ];
        for (code, modifiers, expected) in cases {
            assert_eq!(
                action(Mode::Filter, key(code, modifiers)),
                Some(expected),
                "{code:?} {modifiers:?}"
            );
        }
    }

    #[test]
    fn prompt_editing_keys_do_nothing_in_menus() {
        assert_eq!(action(Mode::Act, key(KeyCode::Char('o'), CTRL)), None);
        assert_eq!(action(Mode::Move, key(KeyCode::Backspace, NONE)), None);
        assert_eq!(
            action(Mode::ConfirmDelete, key(KeyCode::Backspace, NONE)),
            Some(Action::Backspace)
        );
        assert_eq!(
            action(Mode::ConfirmDelete, key(KeyCode::Char('w'), CTRL)),
            None
        );
        assert_eq!(
            action(Mode::Priority, key(KeyCode::Esc, NONE)),
            Some(Action::Cancel)
        );
    }

    #[test]
    fn labels_name_the_first_binding() {
        assert_eq!(label(Action::OpenMenu).as_deref(), Some("ctrl-o"));
        assert_eq!(label(Action::ToggleDown).as_deref(), Some("tab"));
        assert_eq!(label(Action::Cancel).as_deref(), Some("esc"));
        assert_eq!(label(Action::Insert('a')), None);
    }
}

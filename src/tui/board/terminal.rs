use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;

use super::keys;
use super::render;
use super::state::{Board, Effect, Input};
use crate::tui::session::Session;
use crate::tui::state::Response;
use crate::tui::terminal::TtyBackend;

type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

pub(super) fn run(board: &mut Board, session: &mut Session<'_>, color: bool) -> io::Result<()> {
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let screen = Screen::enter(tty.try_clone()?)?;
    let _hook = RestoreOnPanic::install(screen.tty.try_clone()?, Arc::clone(&screen.active));
    let mut terminal = Terminal::new(TtyBackend::new(tty)?)?;
    terminal.clear()?;
    loop {
        terminal.draw(|frame| render::draw(frame, board, color))?;
        if board.quit() {
            return Ok(());
        }
        let inputs = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => keys::inputs(board, key),
            Event::Paste(text) => keys::paste(board, &text),
            _ => continue,
        };
        dispatch(board, inputs, |effect, includes_completed| match effect {
            Effect::Session(next) => Ok(session.perform(next, includes_completed)),
            Effect::Edit(ids) => {
                screen.suspend()?;
                let response = session.edit(&ids, includes_completed);
                screen.resume()?;
                terminal.clear()?;
                Ok(response)
            }
        })?;
    }
}

fn dispatch(
    board: &mut Board,
    inputs: Vec<Input>,
    mut perform: impl FnMut(Effect, bool) -> io::Result<Response>,
) -> io::Result<()> {
    for input in inputs {
        if board.quit() {
            break;
        }
        let mut effect = board.update(input);
        while let Some(next) = effect {
            let response = perform(next, board.includes_completed())?;
            effect = board.resolve(response);
        }
    }
    Ok(())
}

struct Screen {
    tty: File,
    active: Arc<AtomicBool>,
}

impl Screen {
    fn enter(tty: File) -> io::Result<Self> {
        let screen = Self {
            tty,
            active: Arc::new(AtomicBool::new(false)),
        };
        screen.resume()?;
        Ok(screen)
    }

    fn resume(&self) -> io::Result<()> {
        self.active.store(true, Ordering::SeqCst);
        terminal::enable_raw_mode()?;
        let mut tty = &self.tty;
        execute!(tty, EnterAlternateScreen, EnableBracketedPaste, Hide)?;
        tty.flush()
    }

    fn suspend(&self) -> io::Result<()> {
        restore(&self.tty, &self.active)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

fn restore(mut tty: &File, active: &AtomicBool) -> io::Result<()> {
    if !active.swap(false, Ordering::SeqCst) {
        return Ok(());
    }
    all_steps([
        &mut move || execute!(tty, DisableBracketedPaste),
        &mut move || execute!(tty, LeaveAlternateScreen),
        &mut move || execute!(tty, Show),
        &mut move || tty.flush(),
        &mut terminal::disable_raw_mode,
    ])
}

fn all_steps<const N: usize>(steps: [&mut dyn FnMut() -> io::Result<()>; N]) -> io::Result<()> {
    let mut first = Ok(());
    for step in steps {
        let result = step();
        if first.is_ok() {
            first = result;
        }
    }
    first
}

struct RestoreOnPanic {
    previous: Arc<PanicHook>,
}

impl RestoreOnPanic {
    fn install(tty: File, active: Arc<AtomicBool>) -> Self {
        let previous: Arc<PanicHook> = Arc::new(panic::take_hook());
        let chained = Arc::clone(&previous);
        panic::set_hook(Box::new(move |info| {
            let _ = restore(&tty, &active);
            chained(info);
        }));
        Self { previous }
    }
}

impl Drop for RestoreOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        drop(panic::take_hook());
        let previous = std::mem::replace(
            &mut self.previous,
            Arc::new(Box::new(|_: &PanicHookInfo<'_>| {})),
        );
        match Arc::try_unwrap(previous) {
            Ok(previous) => panic::set_hook(previous),
            Err(previous) => panic::set_hook(Box::new(move |info| previous(info))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::keymap::Action;
    use crate::domain::{Config, TaskChange};
    use crate::tui::board::state::tests::board_with;
    use crate::tui::state::Effect as SessionEffect;
    use crate::tui::state::tests::data;

    #[test]
    fn a_sequence_stops_running_once_it_quits() {
        for after in [Action::Done, Action::Edit, Action::Delete] {
            let mut board = board_with(data(), Config::default());
            let mut performed = Vec::new();
            dispatch(
                &mut board,
                vec![Input::Action(Action::Quit), Input::Action(after)],
                |effect, _| {
                    performed.push(effect);
                    Ok(Response::Failed("unexpected".into()))
                },
            )
            .unwrap();
            assert!(board.quit());
            assert_eq!(performed, []);
            assert_eq!(board.overlay(), None);
        }
    }

    #[test]
    fn a_sequence_runs_every_action_until_then() {
        let mut board = board_with(data(), Config::default());
        let mut performed = Vec::new();
        dispatch(
            &mut board,
            vec![
                Input::Action(Action::Done),
                Input::Action(Action::Quit),
                Input::Action(Action::Done),
            ],
            |effect, _| {
                performed.push(effect);
                Ok(Response::Changed {
                    message: "done".into(),
                    error: false,
                    data: Some(data()),
                })
            },
        )
        .unwrap();
        assert_eq!(
            performed,
            [Effect::Session(SessionEffect::Change {
                change: TaskChange::Complete,
                ids: vec!["rem-1".into()],
            })]
        );
    }

    #[test]
    fn every_restore_step_runs_even_when_an_earlier_one_fails() {
        let mut ran = Vec::new();
        let ran = std::cell::RefCell::new(&mut ran);
        let result = all_steps([
            &mut || {
                ran.borrow_mut().push(1);
                Err(io::Error::other("leave failed"))
            },
            &mut || {
                ran.borrow_mut().push(2);
                Err(io::Error::other("flush failed"))
            },
            &mut || {
                ran.borrow_mut().push(3);
                Ok(())
            },
        ]);
        assert_eq!(result.unwrap_err().to_string(), "leave failed");
        assert_eq!(**ran.borrow(), [1, 2, 3]);
    }
}

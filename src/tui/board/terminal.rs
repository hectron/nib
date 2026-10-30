use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

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

static PANIC_HOOK: Mutex<()> = Mutex::new(());

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
    tty: Arc<Mutex<Option<File>>>,
    previous: Option<Arc<PanicHook>>,
    _installed: MutexGuard<'static, ()>,
}

impl RestoreOnPanic {
    fn install(tty: File, active: Arc<AtomicBool>) -> Self {
        let installed = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        let tty = Arc::new(Mutex::new(Some(tty)));
        let previous: Arc<PanicHook> = Arc::new(panic::take_hook());
        let owner = thread::current().id();
        let (hook_tty, chained) = (Arc::clone(&tty), Arc::clone(&previous));
        panic::set_hook(Box::new(move |info| {
            if thread::current().id() == owner
                && let Some(tty) = lock(&hook_tty).as_ref()
            {
                let _ = restore(tty, &active);
            }
            chained(info);
        }));
        Self {
            tty,
            previous: Some(previous),
            _installed: installed,
        }
    }
}

impl Drop for RestoreOnPanic {
    fn drop(&mut self) {
        drop(lock(&self.tty).take());
        if thread::panicking() {
            return;
        }
        drop(panic::take_hook());
        let Some(previous) = self.previous.take() else {
            return;
        };
        match Arc::try_unwrap(previous) {
            Ok(previous) => panic::set_hook(previous),
            Err(previous) => panic::set_hook(Box::new(move |info| previous(info))),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

    static HOOK_TESTS: Mutex<()> = Mutex::new(());
    const MARKER: &str = "board hook test:";

    fn message(info: &PanicHookInfo<'_>) -> String {
        let payload = info.payload();
        payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default()
    }

    fn with_recording_hook(body: impl FnOnce()) -> Vec<String> {
        let _serial = lock(&HOOK_TESTS);
        let original: Arc<PanicHook> = Arc::new(panic::take_hook());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (chained, record) = (Arc::clone(&original), Arc::clone(&seen));
        panic::set_hook(Box::new(move |info| {
            let text = message(info);
            if text.starts_with(MARKER) {
                lock(&record).push(text);
            } else {
                chained(info);
            }
        }));
        let result = panic::catch_unwind(panic::AssertUnwindSafe(body));
        drop(panic::take_hook());
        match Arc::try_unwrap(original) {
            Ok(original) => panic::set_hook(original),
            Err(original) => panic::set_hook(Box::new(move |info| original(info))),
        }
        if let Err(payload) = result {
            panic::resume_unwind(payload);
        }
        lock(&seen).clone()
    }

    fn tty() -> File {
        OpenOptions::new().write(true).open("/dev/null").unwrap()
    }

    fn panic_here(name: &str) {
        let caught = panic::catch_unwind(|| panic!("{MARKER} {name}"));
        assert!(caught.is_err());
    }

    #[test]
    fn the_panic_hook_restores_only_for_the_owner_thread_and_is_removed_on_drop() {
        let seen = with_recording_hook(|| {
            let active = Arc::new(AtomicBool::new(true));
            let hook = RestoreOnPanic::install(tty(), Arc::clone(&active));
            let joined = thread::spawn(|| panic!("{MARKER} other thread")).join();
            assert!(joined.is_err());
            assert!(active.load(Ordering::SeqCst));
            panic_here("owner");
            assert!(!active.load(Ordering::SeqCst));
            drop(hook);
            active.store(true, Ordering::SeqCst);
            panic_here("after drop");
            assert!(active.load(Ordering::SeqCst));
        });
        assert_eq!(
            seen,
            [
                format!("{MARKER} other thread"),
                format!("{MARKER} owner"),
                format!("{MARKER} after drop"),
            ]
        );
    }

    #[test]
    fn a_hook_dropped_while_unwinding_stops_restoring_and_frees_the_slot() {
        let seen = with_recording_hook(|| {
            let active = Arc::new(AtomicBool::new(true));
            let unwound = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let _hook = RestoreOnPanic::install(tty(), Arc::clone(&active));
                panic!("{MARKER} while running");
            }));
            assert!(unwound.is_err());
            assert!(!active.load(Ordering::SeqCst));
            active.store(true, Ordering::SeqCst);
            panic_here("after unwinding");
            assert!(active.load(Ordering::SeqCst));
            let second = Arc::new(AtomicBool::new(true));
            let hook = RestoreOnPanic::install(tty(), Arc::clone(&second));
            panic_here("second board");
            assert!(!second.load(Ordering::SeqCst));
            drop(hook);
        });
        assert_eq!(
            seen,
            [
                format!("{MARKER} while running"),
                format!("{MARKER} after unwinding"),
                format!("{MARKER} second board"),
            ]
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

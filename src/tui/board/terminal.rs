use std::fs::{File, OpenOptions};
use std::io::{self, Write};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;

use super::keys;
use super::render;
use super::state::{Board, Effect};
use crate::tui::session::Session;
use crate::tui::terminal::TtyBackend;

pub(super) fn run(board: &mut Board, session: &mut Session<'_>, color: bool) -> io::Result<()> {
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let screen = Screen::enter(tty.try_clone()?)?;
    let mut terminal = Terminal::new(TtyBackend::new(tty)?)?;
    terminal.clear()?;
    loop {
        terminal.draw(|frame| render::draw(frame, board, color))?;
        if board.quit() {
            return Ok(());
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        for input in keys::inputs(board, key) {
            let mut effect = board.update(input);
            while let Some(next) = effect {
                let includes_completed = board.includes_completed();
                let response = match next {
                    Effect::Session(next) => session.perform(next, includes_completed),
                    Effect::Edit(ids) => {
                        screen.suspend()?;
                        let response = session.edit(&ids, includes_completed);
                        screen.resume()?;
                        terminal.clear()?;
                        response
                    }
                };
                effect = board.resolve(response);
            }
        }
    }
}

struct Screen {
    tty: File,
}

impl Screen {
    fn enter(tty: File) -> io::Result<Self> {
        let screen = Self { tty };
        screen.resume()?;
        Ok(screen)
    }

    fn resume(&self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut tty = &self.tty;
        execute!(tty, EnterAlternateScreen, Hide)?;
        tty.flush()
    }

    fn suspend(&self) -> io::Result<()> {
        let mut tty = &self.tty;
        execute!(tty, LeaveAlternateScreen, Show)?;
        tty.flush()?;
        terminal::disable_raw_mode()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

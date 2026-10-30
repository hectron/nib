use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal;
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Rect, Size};
use ratatui::{Terminal, TerminalOptions, Viewport};
use rustix::event::{PollFd, PollFlags, Timespec};

use super::keys;
use super::render;
use super::session::Session;
use super::state::State;

const MAX_HEIGHT: u16 = 14;
const CURSOR_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn run(state: &mut State, session: &mut Session<'_>, color: bool) -> io::Result<()> {
    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    let query = tty.try_clone()?;
    let _raw = RawMode::enable()?;
    let backend = TtyBackend {
        inner: CrosstermBackend::new(tty),
        query,
    };
    let height = backend.size()?.height.clamp(1, MAX_HEIGHT);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    )?;
    let mut area = Rect::default();
    let result = (|| -> io::Result<()> {
        loop {
            terminal.draw(|frame| {
                area = frame.area();
                render::draw(frame, state, color);
            })?;
            if state.outcome().is_some() {
                return Ok(());
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let Some(action) = keys::action(state.mode(), key) else {
                continue;
            };
            let mut effect = state.update(action);
            while let Some(next) = effect {
                let response = session.perform(next, state.includes_completed());
                effect = state.resolve(response);
            }
        }
    })();
    let backend = terminal.backend_mut();
    backend.set_cursor_position(area.as_position())?;
    backend.clear_region(ClearType::AfterCursor)?;
    backend.show_cursor()?;
    Backend::flush(backend)?;
    result
}

struct RawMode;

impl RawMode {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

struct TtyBackend {
    inner: CrosstermBackend<File>,
    query: File,
}

impl TtyBackend {
    fn query_cursor(&mut self) -> io::Result<Position> {
        self.query.write_all(b"\x1b[6n")?;
        self.query.flush()?;
        let stdin = rustix::stdio::stdin();
        let deadline = Instant::now() + CURSOR_TIMEOUT;
        let mut response = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = Timespec {
                tv_sec: i64::try_from(remaining.as_secs()).unwrap_or(i64::MAX),
                tv_nsec: i64::from(remaining.subsec_nanos()),
            };
            let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
            if remaining.is_zero() || rustix::event::poll(&mut fds, Some(&timeout))? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the terminal didn't report the cursor position",
                ));
            }
            let mut byte = [0_u8; 1];
            if rustix::io::read(stdin, &mut byte)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            response.push(byte[0]);
            if byte[0] == b'R'
                && let Some(position) = parse_cursor_report(&response)
            {
                return Ok(position);
            }
        }
    }
}

fn parse_cursor_report(bytes: &[u8]) -> Option<Position> {
    let start = bytes.windows(2).rposition(|pair| pair == b"\x1b[")?;
    let body = std::str::from_utf8(&bytes[start + 2..bytes.len().checked_sub(1)?]).ok()?;
    let (row, column) = body.split_once(';')?;
    let row: u16 = row.parse().ok()?;
    let column: u16 = column.parse().ok()?;
    Some(Position {
        x: column.checked_sub(1)?,
        y: row.checked_sub(1)?,
    })
}

impl Backend for TtyBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Backend::flush(&mut self.inner)?;
        self.query_cursor().or_else(|_| {
            let size = self.inner.size()?;
            Ok(Position {
                x: 0,
                y: size.height.saturating_sub(1),
            })
        })
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_reports_are_parsed_as_zero_based_positions() {
        assert_eq!(
            parse_cursor_report(b"\x1b[12;5R"),
            Some(Position { x: 4, y: 11 })
        );
        assert_eq!(
            parse_cursor_report(b"typed\x1b[1;1R"),
            Some(Position { x: 0, y: 0 })
        );
        assert_eq!(parse_cursor_report(b"\x1b[0;1R"), None);
        assert_eq!(parse_cursor_report(b"\x1b[R"), None);
    }
}

mod keys;
mod render;
mod state;
mod terminal;

use std::io::{self, IsTerminal};

use jiff::{ToSpan, Zoned};
use miette::{Diagnostic, IntoDiagnostic, Result};
use thiserror::Error;

use crate::cli::output::clean;
use crate::domain::{Backends, Config, Event, EventQuery};
use crate::infra::host::Host;
use crate::tui::session::Session;
use crate::tui::state::Status;
use state::Board;

const EVENT_DAYS: i32 = 7;

#[derive(Debug, Error, Diagnostic)]
pub(crate) enum BoardError {
    #[error("nib board needs an interactive terminal")]
    #[diagnostic(
        code(nib::board::not_a_terminal),
        help(
            "run it with standard input attached to a terminal; scripts can use `nib task list --filter EXPR --json` and `nib cal --week --json` instead"
        )
    )]
    NotInteractive,
}

pub(crate) fn board(
    now: &Zoned,
    config: &Config,
    backends: &Backends,
    host: &mut dyn Host,
    color: bool,
) -> Result<()> {
    if !io::stdin().is_terminal() {
        return Err(BoardError::NotInteractive.into());
    }
    let mut session = Session {
        now,
        config,
        backends,
        host,
    };
    let data = session.load(false)?;
    let (events, status) = match events(now, config, backends) {
        Ok(events) => (events, None),
        Err(error) => (
            Vec::new(),
            Some(Status {
                text: format!(
                    "Couldn't load calendar events: {}",
                    clean(&error.to_string())
                ),
                error: true,
            }),
        ),
    };
    let (mut board, mut effect) = Board::new(data, events, config.clone(), now.clone(), status);
    while let Some(state::Effect::Session(next)) = effect {
        effect = board.resolve(session.perform(next, board.includes_completed()));
    }
    terminal::run(&mut board, &mut session, color).into_diagnostic()
}

fn events(now: &Zoned, config: &Config, backends: &Backends) -> Result<Vec<Event>> {
    let start = now.start_of_day().into_diagnostic()?;
    let end = now
        .date()
        .checked_add(EVENT_DAYS.days())
        .and_then(|end| end.to_zoned(now.time_zone().clone()))
        .into_diagnostic()?;
    Ok(backends.calendar.events(EventQuery {
        range: start..end,
        calendars: config.cal.calendars.clone(),
    })?)
}

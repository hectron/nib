mod board;
mod keys;
mod render;
mod session;
mod state;
mod terminal;

use std::io::{self, IsTerminal};

use jiff::Zoned;
use miette::{Diagnostic, IntoDiagnostic, Result};
use thiserror::Error;

use crate::cli::output::{self, PickedReminder, Style};
use crate::domain::{Backends, Config};
use crate::infra::host::Host;
pub(crate) use board::board;
use session::Session;
use state::{Outcome, State};

#[derive(Debug, Error, Diagnostic)]
pub(crate) enum PickError {
    #[error("nib pick needs an interactive terminal")]
    #[diagnostic(
        code(nib::pick::not_a_terminal),
        help(
            "run it with standard input attached to a terminal; scripts can use `nib task list --filter EXPR --json` instead"
        )
    )]
    NotInteractive,
}

pub(crate) struct Picked {
    pub text: String,
    pub cancelled: bool,
}

pub(crate) struct Request<'a> {
    pub query: &'a str,
    pub json: bool,
    pub style: Style,
    pub color: bool,
}

pub(crate) fn pick(
    request: &Request<'_>,
    now: &Zoned,
    config: &Config,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<Picked> {
    if !io::stdin().is_terminal() {
        return Err(PickError::NotInteractive.into());
    }
    let mut session = Session {
        now,
        config,
        backends,
        host,
    };
    let mut state = start(&mut session, request.query)?;
    terminal::run(&mut state, &mut session, request.color).into_diagnostic()?;
    finish(&state, request.json, request.style)
}

fn start(session: &mut Session<'_>, query: &str) -> Result<State> {
    let (mut state, mut effect) = State::new(
        session.load(false)?,
        query,
        session.now.clone(),
        session.config.task.parse_inline,
    );
    while let Some(next) = effect {
        effect = state.resolve(session.perform(next, state.includes_completed()));
    }
    Ok(state)
}

fn finish(state: &State, json: bool, style: Style) -> Result<Picked> {
    let Some(Outcome::Picked(ids)) = state.outcome() else {
        return Ok(Picked {
            text: String::new(),
            cancelled: true,
        });
    };
    let items: Vec<PickedReminder<'_>> = ids
        .iter()
        .filter_map(|id| state.item(id))
        .map(|item| PickedReminder {
            id: &item.id,
            title: &item.title,
            list: item.list.as_deref(),
            due: item.due,
            due_time: item.due_time,
            due_time_zone: item.due_time_zone.as_deref(),
            priority: item.priority,
            completed: item.completed,
        })
        .collect();
    Ok(Picked {
        text: output::picked(&items, json, style)?,
        cancelled: false,
    })
}

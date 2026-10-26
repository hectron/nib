use std::cell::LazyCell;
use std::ops::Range;

use jiff::Zoned;
use jiff::civil::Date;
use miette::Diagnostic;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod eventkit;

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub due: Option<Date>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: Zoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNote {
    pub title: String,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteQuery {
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReminder {
    pub title: String,
    pub due: Option<Date>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQuery {
    pub range: Range<Zoned>,
}

// The stubs only return NotImplemented; the other variants are for the Apple backends.
#[derive(Debug, Error, Diagnostic)]
#[allow(dead_code)]
pub enum BackendError {
    #[error("{terminal} doesn't have access to {app}")]
    #[diagnostic(
        code(nib::backend::permission_denied),
        help(
            "turn on {terminal} under System Settings > Privacy & Security > {app}, then run the command again; the VS Code terminal, ssh sessions, and cron jobs can't show the permission prompt"
        )
    )]
    PermissionDenied { app: &'static str, terminal: String },

    #[error("{kind} \"{name}\" not found")]
    #[diagnostic(
        code(nib::backend::not_found),
        help("check the spelling and try again")
    )]
    NotFound { kind: &'static str, name: String },

    #[error("{app} integration is not implemented yet")]
    #[diagnostic(
        code(nib::backend::not_implemented),
        help("only the command-line interface exists so far")
    )]
    NotImplemented { app: &'static str },

    #[error("{app} request failed: {message}")]
    #[diagnostic(
        code(nib::backend::failed),
        help("try again; if it keeps failing, make sure {app} opens normally")
    )]
    Failed { app: &'static str, message: String },
}

pub fn terminal_name(term_program: Option<&str>) -> String {
    match term_program {
        Some("Apple_Terminal") => "Terminal".into(),
        Some("iTerm.app") => "iTerm".into(),
        Some("vscode") => "Visual Studio Code".into(),
        Some("ghostty") => "Ghostty".into(),
        Some(name) if !name.is_empty() => name.into(),
        _ => "Your terminal app".into(),
    }
}

pub trait Notes {
    fn create(&self, note: NewNote) -> Result<Note, BackendError>;
    fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError>;
}

pub trait Reminders {
    fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError>;
    fn list(&self) -> Result<Vec<Reminder>, BackendError>;
}

pub trait Calendar {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError>;
}

pub type Lazy<T> = LazyCell<Box<T>, Box<dyn FnOnce() -> Box<T>>>;

pub struct Backends {
    pub notes: Lazy<dyn Notes>,
    pub reminders: Lazy<dyn Reminders>,
    pub calendar: Lazy<dyn Calendar>,
}

impl Backends {
    pub fn new(
        notes: impl FnOnce() -> Box<dyn Notes> + 'static,
        reminders: impl FnOnce() -> Box<dyn Reminders> + 'static,
        calendar: impl FnOnce() -> Box<dyn Calendar> + 'static,
    ) -> Self {
        Self {
            notes: LazyCell::new(Box::new(notes)),
            reminders: LazyCell::new(Box::new(reminders)),
            calendar: LazyCell::new(Box::new(calendar)),
        }
    }

    pub fn system() -> Self {
        Self::new(
            || Box::new(Unimplemented("Apple Notes")),
            system_reminders,
            system_calendar,
        )
    }
}

#[cfg(target_os = "macos")]
fn system_calendar() -> Box<dyn Calendar> {
    Box::new(eventkit::EventKitCalendar)
}

#[cfg(not(target_os = "macos"))]
fn system_calendar() -> Box<dyn Calendar> {
    Box::new(Unimplemented("Calendar"))
}

#[cfg(target_os = "macos")]
fn system_reminders() -> Box<dyn Reminders> {
    Box::new(eventkit::EventKitReminders)
}

#[cfg(not(target_os = "macos"))]
fn system_reminders() -> Box<dyn Reminders> {
    Box::new(Unimplemented("Reminders"))
}

struct Unimplemented(&'static str);

impl Unimplemented {
    fn fail<T>(&self) -> Result<T, BackendError> {
        Err(BackendError::NotImplemented { app: self.0 })
    }
}

impl Notes for Unimplemented {
    fn create(&self, _: NewNote) -> Result<Note, BackendError> {
        self.fail()
    }

    fn list(&self, _: NoteQuery) -> Result<Vec<Note>, BackendError> {
        self.fail()
    }
}

impl Reminders for Unimplemented {
    fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
        self.fail()
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        self.fail()
    }
}

impl Calendar for Unimplemented {
    fn events(&self, _: EventQuery) -> Result<Vec<Event>, BackendError> {
        self.fail()
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn errors_name_the_code_and_the_fix() {
        let errors = [
            BackendError::PermissionDenied {
                app: "Reminders",
                terminal: "WezTerm".into(),
            },
            BackendError::NotFound {
                kind: "folder",
                name: "Archive".into(),
            },
            BackendError::NotImplemented { app: "Calendar" },
            BackendError::Failed {
                app: "Reminders",
                message: "timed out".into(),
            },
        ];
        let rendered: Vec<String> = errors
            .iter()
            .map(|error| {
                format!(
                    "{}: {error}\n  help: {}",
                    error.code().unwrap(),
                    error.help().unwrap()
                )
            })
            .collect();
        assert_snapshot!(rendered.join("\n"), @r#"
        nib::backend::permission_denied: WezTerm doesn't have access to Reminders
          help: turn on WezTerm under System Settings > Privacy & Security > Reminders, then run the command again; the VS Code terminal, ssh sessions, and cron jobs can't show the permission prompt
        nib::backend::not_found: folder "Archive" not found
          help: check the spelling and try again
        nib::backend::not_implemented: Calendar integration is not implemented yet
          help: only the command-line interface exists so far
        nib::backend::failed: Reminders request failed: timed out
          help: try again; if it keeps failing, make sure Reminders opens normally
        "#);
    }

    #[test]
    fn terminal_names_are_readable() {
        assert_eq!(terminal_name(Some("Apple_Terminal")), "Terminal");
        assert_eq!(terminal_name(Some("iTerm.app")), "iTerm");
        assert_eq!(terminal_name(Some("WezTerm")), "WezTerm");
        assert_eq!(terminal_name(Some("")), "Your terminal app");
        assert_eq!(terminal_name(None), "Your terminal app");
    }
}

use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    pub title: String,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reminder {
    pub title: String,
    pub due: Option<String>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub title: String,
    pub start: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Day,
    Week,
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
    pub due: Option<String>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventQuery {
    pub span: Span,
}

// The stubs only return NotImplemented; the other variants are for the Apple backends.
#[derive(Debug, Error, Diagnostic)]
#[allow(dead_code)]
pub enum BackendError {
    #[error("{app} permission denied")]
    #[diagnostic(
        code(jot::backend::permission_denied),
        help("allow jot in System Settings > Privacy & Security, then run the command again")
    )]
    PermissionDenied { app: &'static str },

    #[error("{kind} \"{name}\" not found")]
    #[diagnostic(
        code(jot::backend::not_found),
        help("check the spelling and try again")
    )]
    NotFound { kind: &'static str, name: String },

    #[error("{app} integration is not implemented yet")]
    #[diagnostic(
        code(jot::backend::not_implemented),
        help("only the command-line interface exists so far")
    )]
    NotImplemented { app: &'static str },

    #[error("{app} request failed: {message}")]
    #[diagnostic(
        code(jot::backend::failed),
        help("try again; if it keeps failing, make sure {app} opens normally")
    )]
    Failed { app: &'static str, message: String },
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

pub struct Backends {
    pub notes: Box<dyn Notes>,
    pub reminders: Box<dyn Reminders>,
    pub calendar: Box<dyn Calendar>,
}

impl Backends {
    pub fn system() -> Self {
        Self {
            notes: Box::new(Unimplemented("Apple Notes")),
            reminders: Box::new(Unimplemented("Reminders")),
            calendar: Box::new(Unimplemented("Calendar")),
        }
    }
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
            BackendError::PermissionDenied { app: "Reminders" },
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
        jot::backend::permission_denied: Reminders permission denied
          help: allow jot in System Settings > Privacy & Security, then run the command again
        jot::backend::not_found: folder "Archive" not found
          help: check the spelling and try again
        jot::backend::not_implemented: Calendar integration is not implemented yet
          help: only the command-line interface exists so far
        jot::backend::failed: Reminders request failed: timed out
          help: try again; if it keeps failing, make sure Reminders opens normally
        "#);
    }
}

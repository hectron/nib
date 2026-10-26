use miette::{Result, miette};
use serde::Serialize;

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

pub trait Notes {
    fn create(&self, note: NewNote) -> Result<Note>;
    fn list(&self, query: NoteQuery) -> Result<Vec<Note>>;
}

pub trait Reminders {
    fn add(&self, reminder: NewReminder) -> Result<Reminder>;
    fn list(&self) -> Result<Vec<Reminder>>;
}

pub trait Calendar {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>>;
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
    fn fail<T>(&self) -> Result<T> {
        Err(miette!(
            help = "only the command-line interface exists so far",
            "{} integration is not implemented yet",
            self.0
        ))
    }
}

impl Notes for Unimplemented {
    fn create(&self, _: NewNote) -> Result<Note> {
        self.fail()
    }

    fn list(&self, _: NoteQuery) -> Result<Vec<Note>> {
        self.fail()
    }
}

impl Reminders for Unimplemented {
    fn add(&self, _: NewReminder) -> Result<Reminder> {
        self.fail()
    }

    fn list(&self) -> Result<Vec<Reminder>> {
        self.fail()
    }
}

impl Calendar for Unimplemented {
    fn events(&self, _: EventQuery) -> Result<Vec<Event>> {
        self.fail()
    }
}

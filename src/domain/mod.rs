use std::cell::LazyCell;
use std::ops::Range;

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub folder: Option<String>,
    pub modified: Option<Timestamp>,
    pub locked: bool,
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
    pub all_day: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub task: TaskConfig,
    pub cal: CalendarConfig,
    pub note: NoteConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskConfig {
    pub list: Option<String>,
    pub priority: Option<Priority>,
    pub parse_inline: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            list: None,
            priority: None,
            parse_inline: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarConfig {
    pub calendars: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteConfig {
    pub folder: Option<String>,
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
    pub list: Option<String>,
    pub due: Option<Date>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventQuery {
    pub range: Range<Zoned>,
    pub calendars: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderList {
    pub id: String,
    pub name: String,
    pub writable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderDueComponents {
    pub calendar: Option<String>,
    pub time_zone: Option<String>,
    pub era: Option<i64>,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub hour: Option<i64>,
    pub minute: Option<i64>,
    pub second: Option<i64>,
    pub nanosecond: Option<i64>,
    pub weekday: Option<i64>,
    pub weekday_ordinal: Option<i64>,
    pub quarter: Option<i64>,
    pub week_of_month: Option<i64>,
    pub week_of_year: Option<i64>,
    pub year_for_week_of_year: Option<i64>,
    pub day_of_year: Option<i64>,
    pub is_leap_month: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReminder {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<Date>,
    pub due_components: Option<ReminderDueComponents>,
    pub priority: Option<Priority>,
    pub completed: bool,
    pub modified: Option<Timestamp>,
    pub alarms_present: bool,
    pub recurrence_present: bool,
    pub url: Option<String>,
    pub backup_partial: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderSnapshot {
    pub lists: Vec<ReminderList>,
    pub reminders: Vec<StoredReminder>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderEdit {
    pub title: Option<String>,
    pub notes: FieldEdit<String>,
    pub due: FieldEdit<Date>,
    pub due_components: Option<Box<ReminderDueComponents>>,
    pub priority: FieldEdit<Priority>,
    pub list_id: Option<String>,
    pub completed: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldEdit<T> {
    #[default]
    Unchanged,
    Set(T),
    Remove,
}

impl<T> FieldEdit<T> {
    pub fn is_changed(&self) -> bool {
        !matches!(self, Self::Unchanged)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderChange {
    Add {
        list_id: String,
        title: String,
        notes: Option<String>,
        due: Option<Date>,
        priority: Option<Priority>,
    },
    Restore {
        before: StoredReminder,
    },
    Update {
        before: StoredReminder,
        edit: ReminderEdit,
    },
    Delete {
        before: StoredReminder,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyResult {
    pub written: Vec<Option<StoredReminder>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeCounts {
    pub added: usize,
    pub updated: usize,
    pub completed: usize,
    pub deleted: usize,
}

impl ChangeCounts {
    pub fn total(self) -> usize {
        self.added + self.updated + self.completed + self.deleted
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalSummary {
    pub id: String,
    pub time: Timestamp,
    pub action: JournalAction,
    pub undo_of: Option<String>,
    pub undone: bool,
    pub counts: ChangeCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalAction {
    TaskEdit,
    Undo,
}

impl JournalAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskEdit => "task_edit",
            Self::Undo => "undo",
        }
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum BackendError {
    #[error("{terminal} doesn't have access to {app}")]
    #[diagnostic(
        code(nib::backend::permission_denied),
        help(
            "turn on {terminal} under System Settings > Privacy & Security > {app}, then run the command again; the VS Code terminal, ssh sessions, and cron jobs can't show the permission prompt"
        )
    )]
    PermissionDenied { app: &'static str, terminal: String },

    #[error("{terminal} doesn't have permission to control {app}")]
    #[diagnostic(
        code(nib::backend::permission_denied),
        help(
            "turn on {terminal} under System Settings > Privacy & Security > Automation, then run the command again"
        )
    )]
    AutomationDenied { app: &'static str, terminal: String },

    #[error("{kind} \"{name}\" not found")]
    #[diagnostic(
        code(nib::backend::not_found),
        help("check the spelling and try again")
    )]
    NotFound { kind: &'static str, name: String },

    #[error("more than one {kind} is named \"{name}\"")]
    #[diagnostic(
        code(nib::backend::ambiguous),
        help("rename one of them, or choose a unique name")
    )]
    Ambiguous { kind: &'static str, name: String },

    #[cfg(any(not(target_os = "macos"), test))]
    #[error("{app} integration is not implemented yet")]
    #[diagnostic(
        code(nib::backend::not_implemented),
        help("only the command-line interface exists so far")
    )]
    NotImplemented { app: &'static str },

    #[error("{} changed in Reminders since nib last read them", .titles.join(", "))]
    #[diagnostic(
        code(nib::backend::conflict),
        help("nothing was changed; review the named reminders and try the command again")
    )]
    Conflict { titles: Vec<String> },

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
    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError>;
    fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError>;
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
            BackendError::AutomationDenied {
                app: "Apple Notes",
                terminal: "WezTerm".into(),
            },
            BackendError::NotFound {
                kind: "folder",
                name: "Archive".into(),
            },
            BackendError::Ambiguous {
                kind: "writable reminder list",
                name: "Work".into(),
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
        nib::backend::permission_denied: WezTerm doesn't have permission to control Apple Notes
          help: turn on WezTerm under System Settings > Privacy & Security > Automation, then run the command again
        nib::backend::not_found: folder "Archive" not found
          help: check the spelling and try again
        nib::backend::ambiguous: more than one writable reminder list is named "Work"
          help: rename one of them, or choose a unique name
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

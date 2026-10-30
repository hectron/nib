use std::cell::LazyCell;
use std::ops::Range;
use std::path::PathBuf;

use jiff::civil::{Date, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Timestamp, Zoned};
use miette::Diagnostic;
use thiserror::Error;

fn ambiguity_details(matches: &[String]) -> String {
    if matches.is_empty() {
        String::new()
    } else {
        format!(" (matches: {})", matches.join(", "))
    }
}

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
    pub due_time: Option<Time>,
    pub due_time_zone: Option<String>,
    pub due_at: Option<Timestamp>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: Zoned,
    pub all_day: bool,
    pub calendar: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFolder {
    pub id: String,
    pub name: String,
    pub account: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewResource {
    pub name: String,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceSelector {
    Id(String),
    Name { name: String, owner: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarKind {
    Local,
    CalDav,
    Exchange,
    Subscription,
    Birthday,
    Unknown,
}

impl CalendarKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::CalDav => "caldav",
            Self::Exchange => "exchange",
            Self::Subscription => "subscription",
            Self::Birthday => "birthday",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderListInfo {
    pub id: String,
    pub name: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub kind: CalendarKind,
    pub writable: bool,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: String,
    pub name: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub kind: CalendarKind,
    pub writable: bool,
    pub default: bool,
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
    pub show_calendar: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteConfig {
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    TaskList,
    TaskPriority,
    TaskParseInline,
    CalCalendars,
    CalShowCalendar,
    NoteFolder,
}

impl ConfigKey {
    pub const ALL: [Self; 6] = [
        Self::TaskList,
        Self::TaskPriority,
        Self::TaskParseInline,
        Self::CalCalendars,
        Self::CalShowCalendar,
        Self::NoteFolder,
    ];

    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "task.list" => Some(Self::TaskList),
            "task.priority" => Some(Self::TaskPriority),
            "task.parse_inline" => Some(Self::TaskParseInline),
            "cal.calendars" => Some(Self::CalCalendars),
            "cal.show_calendar" => Some(Self::CalShowCalendar),
            "note.folder" => Some(Self::NoteFolder),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskList => "task.list",
            Self::TaskPriority => "task.priority",
            Self::TaskParseInline => "task.parse_inline",
            Self::CalCalendars => "cal.calendars",
            Self::CalShowCalendar => "cal.show_calendar",
            Self::NoteFolder => "note.folder",
        }
    }

    pub fn parts(self) -> (&'static str, &'static str) {
        self.as_str()
            .split_once('.')
            .expect("configuration keys always contain a section separator")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    Unset,
    String(String),
    Priority(Priority),
    Bool(bool),
    Strings(Vec<String>),
}

impl ConfigValue {
    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOrigin {
    Default,
    File(PathBuf),
    Environment(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub key: ConfigKey,
    pub value: ConfigValue,
    pub origin: ConfigOrigin,
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
    pub due_time: Option<Time>,
    pub due_time_zone: Option<String>,
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
    pub absolute_alarms: Option<Vec<Timestamp>>,
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
        due_time: Option<Time>,
        due_time_zone: Option<String>,
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
    pub unrecorded: Vec<bool>,
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
    pub written_at: Timestamp,
    pub action: JournalAction,
    pub status: JournalStatus,
    pub undo_of: Option<String>,
    pub undone: bool,
    pub counts: ChangeCounts,
    pub unrecorded: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalStatus {
    Applied,
    Pending,
}

impl JournalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Pending => "pending",
        }
    }
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

    #[error("{kind} \"{name}\" already exists in {owner}")]
    #[diagnostic(
        code(nib::backend::already_exists),
        help("choose a unique name or rename the existing resource")
    )]
    AlreadyExists {
        kind: &'static str,
        name: String,
        owner: String,
    },

    #[error("{kind} \"{name}\" can't be renamed because it is immutable")]
    #[diagnostic(
        code(nib::backend::immutable),
        help("choose a resource that can be changed in its Apple app")
    )]
    Immutable { kind: &'static str, name: String },

    #[error("{app} doesn't support {operation}")]
    #[diagnostic(
        code(nib::backend::unsupported),
        help("use the Apple app to make this change")
    )]
    Unsupported {
        app: &'static str,
        operation: &'static str,
    },

    #[error("{kind} \"{name}\" not found")]
    #[diagnostic(
        code(nib::backend::not_found),
        help("check the spelling and try again")
    )]
    NotFound { kind: &'static str, name: String },

    #[error(
        "more than one {kind} is named \"{name}\"{details}",
        details = ambiguity_details(.matches)
    )]
    #[diagnostic(
        code(nib::backend::ambiguous),
        help("rename one of them, or choose a unique name")
    )]
    Ambiguous {
        kind: &'static str,
        name: String,
        matches: Vec<String>,
    },

    #[error("note title contains whitespace that Apple Notes can't preserve exactly")]
    #[diagnostic(
        code(nib::notes::unsupported_whitespace),
        help(
            "use single spaces between words and remove tabs, line breaks, and leading, trailing, or repeated spaces"
        )
    )]
    UnsupportedNoteWhitespace,

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionStatus {
    FullAccess,
    NotDetermined,
    Restricted,
    Denied,
    WriteOnly,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationStatus {
    Allowed,
    Denied,
    NotDetermined,
    TargetNotRunning,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSetting {
    pub variable: Option<&'static str>,
    pub command: Option<String>,
}

pub trait DoctorProbe {
    fn macos_version(&self) -> Result<String, String>;
    fn terminal(&self) -> (String, bool);
    fn reminders_authorization(&self) -> PermissionStatus;
    fn calendars_authorization(&self) -> PermissionStatus;
    fn notes_automation(&self) -> AutomationStatus;
    fn editor(&self) -> EditorSetting;
}

pub trait Notes {
    fn create(&self, note: NewNote) -> Result<Note, BackendError>;
    fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError>;
    fn folders(&self) -> Result<Vec<NoteFolder>, BackendError> {
        Err(BackendError::Unsupported {
            app: "Apple Notes",
            operation: "listing folders",
        })
    }
    fn create_folder(&self, _folder: NewResource) -> Result<NoteFolder, BackendError> {
        Err(BackendError::Unsupported {
            app: "Apple Notes",
            operation: "creating folders",
        })
    }
    fn rename_folder(
        &self,
        _folder: ResourceSelector,
        _name: String,
    ) -> Result<NoteFolder, BackendError> {
        Err(BackendError::Unsupported {
            app: "Apple Notes",
            operation: "renaming folders",
        })
    }
}

pub trait Reminders {
    fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError>;
    fn list(&self) -> Result<Vec<Reminder>, BackendError>;
    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError>;
    fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
        self.snapshot(None)
    }
    fn lists(&self) -> Result<Vec<ReminderListInfo>, BackendError> {
        self.snapshot(None).map(|snapshot| {
            snapshot
                .lists
                .into_iter()
                .map(|list| ReminderListInfo {
                    id: list.id,
                    name: list.name,
                    source: None,
                    source_id: None,
                    kind: CalendarKind::Unknown,
                    writable: list.writable,
                    default: false,
                })
                .collect()
        })
    }
    fn create_list(&self, _list: NewResource) -> Result<ReminderListInfo, BackendError> {
        Err(BackendError::Unsupported {
            app: "Reminders",
            operation: "creating lists",
        })
    }
    fn rename_list(
        &self,
        _list: ResourceSelector,
        _name: String,
    ) -> Result<ReminderListInfo, BackendError> {
        Err(BackendError::Unsupported {
            app: "Reminders",
            operation: "renaming lists",
        })
    }
    fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError>;
    fn validate_time_zone(&self, _name: &str) -> Result<(), BackendError> {
        Ok(())
    }
}

pub fn local_time_zone_name_at(time_zone: &TimeZone, date: Date, time: Time) -> String {
    if let Some(name) = time_zone.iana_name() {
        return name.to_owned();
    }
    fixed_offset_name_at(time_zone, date, time)
}

pub fn time_zone_is_local_at(name: &str, time_zone: &TimeZone, date: Date, time: Time) -> bool {
    if time_zone.iana_name() == Some(name) {
        return true;
    }
    name == fixed_offset_name_at(time_zone, date, time)
}

fn fixed_offset_name_at(time_zone: &TimeZone, date: Date, time: Time) -> String {
    let offset = match time_zone
        .to_ambiguous_timestamp(date.to_datetime(time))
        .offset()
    {
        AmbiguousOffset::Unambiguous { offset }
        | AmbiguousOffset::Gap { before: offset, .. }
        | AmbiguousOffset::Fold { before: offset, .. } => offset,
    };
    let seconds = offset.seconds();
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.unsigned_abs() / 60;
    format!("GMT{sign}{:02}{:02}", minutes / 60, minutes % 60)
}

pub trait Calendar {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError>;
    fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
        Err(BackendError::Unsupported {
            app: "Calendar",
            operation: "listing calendars",
        })
    }
    fn create_calendar(&self, _calendar: NewResource) -> Result<CalendarInfo, BackendError> {
        Err(BackendError::Unsupported {
            app: "Calendar",
            operation: "creating calendars",
        })
    }
    fn rename_calendar(
        &self,
        _calendar: ResourceSelector,
        _name: String,
    ) -> Result<CalendarInfo, BackendError> {
        Err(BackendError::Unsupported {
            app: "Calendar",
            operation: "renaming calendars",
        })
    }
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
    use jiff::civil::{date, time};

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
                matches: Vec::new(),
            },
            BackendError::UnsupportedNoteWhitespace,
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
        nib::notes::unsupported_whitespace: note title contains whitespace that Apple Notes can't preserve exactly
          help: use single spaces between words and remove tabs, line breaks, and leading, trailing, or repeated spaces
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

    #[test]
    fn unnamed_zone_gaps_use_the_offset_before_the_transition() {
        let zone = TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap();

        assert_eq!(
            local_time_zone_name_at(&zone, date(2027, 3, 14), time(2, 30, 0, 0)),
            "GMT-0600"
        );
    }

    #[test]
    fn unnamed_zone_repeated_times_use_the_offset_before_the_transition() {
        let zone = TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap();

        assert_eq!(
            local_time_zone_name_at(&zone, date(2027, 11, 7), time(1, 30, 0, 0)),
            "GMT-0500"
        );
    }

    #[test]
    fn time_zone_is_local_at_matches_gap_and_repeated_times() {
        let zone = TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap();

        assert!(time_zone_is_local_at(
            "GMT-0600",
            &zone,
            date(2027, 3, 14),
            time(2, 30, 0, 0)
        ));
        assert!(time_zone_is_local_at(
            "GMT-0500",
            &zone,
            date(2027, 11, 7),
            time(1, 30, 0, 0)
        ));
        assert!(!time_zone_is_local_at(
            "GMT-0600",
            &zone,
            date(2027, 11, 7),
            time(1, 30, 0, 0)
        ));
    }
}

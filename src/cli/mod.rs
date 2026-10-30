use std::ops::Range;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;
use jiff::civil::Date;
use jiff::{ToSpan, Zoned};
use thiserror::Error;

use crate::domain::{
    Config, EventQuery, NewNote, NewReminder, NewResource, NoteQuery, Priority, ResourceSelector,
    TaskChangeRequest,
};

pub(crate) mod completions;
pub mod output;
mod quick;
mod task;

use task::{Due, InlineTask, due, parse_inline};

/// Fast CLI for Apple Notes, Reminders, and Calendar
#[derive(Debug, Parser)]
#[command(name = "nib", version)]
pub struct Cli {
    /// Print JSON instead of text
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Doctor,
    Config(ConfigAction),
    Today {
        date: Date,
        query: EventQuery,
        show_calendar: bool,
    },
    NewNote(NewNote),
    ListNotes(NoteQuery),
    ListNoteFolders,
    CreateNoteFolder(NewResource),
    RenameNoteFolder {
        folder: ResourceSelector,
        name: String,
    },
    AddTask(NewReminder),
    ListTasks,
    FilterTasks(String),
    ListReminderLists,
    CreateReminderList(NewResource),
    RenameReminderList {
        list: ResourceSelector,
        name: String,
    },
    EditTasks {
        list: Option<String>,
    },
    ChangeTasks(TaskChangeRequest),
    ListEvents {
        query: EventQuery,
        show_calendar: bool,
    },
    ListCalendars,
    CreateCalendar(NewResource),
    RenameCalendar {
        calendar: ResourceSelector,
        name: String,
    },
    Log,
    Undo {
        entry: Option<String>,
        resolve: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(name = "__complete", hide = true)]
    Complete {
        #[arg(long)]
        refresh: bool,
        #[arg(value_enum)]
        kind: CompletionKind,
    },
    /// Generate a shell completion script
    Completions {
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Check system access and configuration without prompting
    Doctor,
    /// Inspect and edit configuration
    Config(ConfigArgs),
    /// Show today's reminders and events
    Today(TodayArgs),
    /// Create and list notes
    Note(NoteArgs),
    /// Add, list, and change reminders
    #[command(visible_alias = "todo")]
    Task(TaskArgs),
    /// Show calendar events
    #[command(visible_alias = "c")]
    Cal(CalArgs),
    /// Create a note from the rest of the line, or list notes
    N(NoteCapture),
    /// Add a reminder from the rest of the line, or list reminders
    T(TaskCapture),
    /// List recent task change and undo journal entries
    Log,
    /// Undo a journal entry, or the latest entry when omitted
    Undo(UndoArgs),
}

#[derive(Debug, Args)]
pub struct UndoArgs {
    /// Journal entry identifier from `nib log`
    #[arg(value_name = "ENTRY", conflicts_with = "resolve")]
    entry: Option<String>,

    /// Resolve an interrupted undo journal entry
    #[arg(long, value_name = "ID")]
    resolve: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum ConfigAction {
    /// List effective settings
    List {
        /// Show where each value came from
        #[arg(long)]
        origin: bool,
    },
    /// Print one effective setting
    Get { key: String },
    /// Set a value in a configuration file
    Set {
        key: String,
        value: String,
        #[command(flatten)]
        target: ConfigTargetArgs,
    },
    /// Remove a value from a configuration file
    Unset {
        key: String,
        #[command(flatten)]
        target: ConfigTargetArgs,
    },
    /// Print the user configuration path
    Path,
}

#[derive(Debug, Clone, Copy, Default, Args, PartialEq, Eq)]
#[group(multiple = false)]
pub struct ConfigTargetArgs {
    /// Write .nib.local.toml in the current directory
    #[arg(long)]
    pub local: bool,
    /// Write nib.toml in the current directory
    #[arg(long)]
    pub project: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Zsh,
    Bash,
    Fish,
}

impl From<CompletionShell> for Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Fish => Self::Fish,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionKind {
    ReminderLists,
    WritableReminderLists,
    Calendars,
    NoteFolders,
}

impl CompletionKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReminderLists => "reminder-lists",
            Self::WritableReminderLists => "writable-reminder-lists",
            Self::Calendars => "calendars",
            Self::NoteFolders => "note-folders",
        }
    }
}

#[derive(Debug, Args)]
pub struct TodayArgs {
    /// Only show events from this calendar
    #[arg(long, value_name = "NAME")]
    calendar: Vec<String>,
    /// Show each event's calendar
    #[arg(long)]
    show_calendar: bool,
}

#[derive(Debug, Args)]
pub struct NoteArgs {
    #[command(subcommand)]
    action: Option<NoteAction>,
    #[command(flatten)]
    folder: Folder,
}

#[derive(Debug, Subcommand)]
enum NoteAction {
    /// Create a note
    New(Title),
    /// List notes (the default)
    #[command(visible_alias = "ls")]
    List,
    /// List and manage note folders
    Folders(NoteFoldersArgs),
}

#[derive(Debug, Args)]
struct NoteFoldersArgs {
    #[command(subcommand)]
    action: Option<NoteFolderAction>,
}

#[derive(Debug, Subcommand)]
enum NoteFolderAction {
    /// Create a note folder
    Create {
        #[arg(value_parser = resource_value)]
        name: String,
        /// Account name or identifier; defaults to the Notes default account
        #[arg(long, value_parser = resource_value)]
        account: Option<String>,
    },
    /// Rename a note folder
    Rename {
        #[arg(value_parser = resource_value)]
        resource: String,
        #[arg(value_parser = resource_value)]
        new_name: String,
        /// Interpret `RESOURCE` as a folder identifier instead of a name
        #[arg(long, conflicts_with = "account")]
        id: bool,
        /// Account name or identifier used to disambiguate `RESOURCE`
        #[arg(long, value_parser = resource_value)]
        account: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct NoteCapture {
    /// Title of the new note; lists notes when omitted
    #[arg(value_name = "TITLE", value_parser = word)]
    words: Vec<String>,
    #[command(flatten)]
    folder: Folder,
}

#[derive(Debug, Args)]
struct Folder {
    /// Folder to create in or list from
    #[arg(short, long, global = true)]
    folder: Option<String>,
}

#[derive(Debug, Args)]
pub struct TaskArgs {
    #[command(subcommand)]
    action: Option<TaskAction>,
    /// Filter reminders by text or fields
    #[arg(short, long, value_name = "EXPR")]
    filter: Option<String>,
}

#[derive(Debug, Subcommand)]
enum TaskAction {
    /// Add a reminder
    Add {
        #[command(flatten)]
        title: Title,
        #[command(flatten)]
        opts: TaskOpts,
    },
    /// List reminders (the default)
    #[command(visible_alias = "ls")]
    List {
        /// Filter reminders by text or fields
        #[arg(short, long, value_name = "EXPR")]
        filter: Option<String>,
    },
    /// List and manage reminder lists
    Lists(ReminderListsArgs),
    /// Edit reminders as a Markdown file in $VISUAL or $EDITOR
    Edit {
        /// Only edit this list
        #[arg(short, long)]
        list: Option<String>,
    },
    /// Mark reminders completed
    Done(quick::Selectors),
    /// Mark completed reminders open again
    Reopen(quick::Selectors),
    /// Set or clear the priority of reminders
    Priority {
        #[arg(value_enum)]
        level: quick::PriorityLevel,
        #[command(flatten)]
        targets: quick::Selectors,
    },
    /// Move reminders to another list
    Move {
        /// Name of the list to move to
        #[arg(value_name = "LIST", value_parser = resource_value)]
        destination: String,
        #[command(flatten)]
        targets: quick::Selectors,
    },
}

#[derive(Debug, Args)]
struct ReminderListsArgs {
    #[command(subcommand)]
    action: Option<ReminderListAction>,
}

#[derive(Debug, Subcommand)]
enum ReminderListAction {
    /// Create a reminder list
    Create {
        #[arg(value_parser = resource_value)]
        name: String,
        /// `EventKit` source name or identifier; defaults to the current default list's source
        #[arg(long, value_parser = resource_value)]
        source: Option<String>,
    },
    /// Rename a reminder list
    Rename {
        #[arg(value_parser = resource_value)]
        resource: String,
        #[arg(value_parser = resource_value)]
        new_name: String,
        /// Interpret `RESOURCE` as a list identifier instead of a name
        #[arg(long, conflicts_with = "source")]
        id: bool,
        /// `EventKit` source name or identifier used to disambiguate `RESOURCE`
        #[arg(long, value_parser = resource_value)]
        source: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct TaskCapture {
    /// Title of the new reminder; lists reminders when omitted
    #[arg(value_name = "TITLE", value_parser = word)]
    words: Vec<String>,
    #[command(flatten)]
    opts: TaskOpts,
}

#[derive(Debug, Args)]
struct TaskOpts {
    /// Reminder list to add to
    #[arg(short, long, value_name = "NAME", requires = "words")]
    list: Option<String>,
    /// Due date and optional time
    #[arg(short, long, value_name = "DUE", value_parser = due, requires = "words")]
    due: Option<Due>,
    #[arg(short, long, value_enum, requires = "words")]
    priority: Option<Priority>,
}

#[derive(Debug, Args)]
pub struct CalArgs {
    #[command(subcommand)]
    action: Option<CalAction>,
    /// Show the next seven days instead of today
    #[arg(short, long, global = true)]
    week: bool,
    /// Only show events from this calendar
    #[arg(long, value_name = "NAME", global = true)]
    calendar: Vec<String>,
    /// Show each event's calendar
    #[arg(long, global = true)]
    show_calendar: bool,
}

#[derive(Debug, Subcommand)]
enum CalAction {
    /// List events (the default)
    #[command(visible_alias = "ls")]
    List,
    /// List and manage calendars
    Calendars(CalendarsArgs),
}

#[derive(Debug, Args)]
struct CalendarsArgs {
    #[command(subcommand)]
    action: Option<CalendarResourceAction>,
}

#[derive(Debug, Subcommand)]
enum CalendarResourceAction {
    /// Create a calendar
    Create {
        #[arg(value_parser = resource_value)]
        name: String,
        /// `EventKit` source name or identifier; defaults to the current default calendar's source
        #[arg(long, value_parser = resource_value)]
        source: Option<String>,
    },
    /// Rename a calendar
    Rename {
        #[arg(value_parser = resource_value)]
        resource: String,
        #[arg(value_parser = resource_value)]
        new_name: String,
        /// Interpret `RESOURCE` as a calendar identifier instead of a name
        #[arg(long, conflicts_with = "source")]
        id: bool,
        /// `EventKit` source name or identifier used to disambiguate `RESOURCE`
        #[arg(long, value_parser = resource_value)]
        source: Option<String>,
    },
}

#[derive(Debug, Args)]
struct Title {
    /// Title; the words are joined with spaces
    #[arg(value_name = "TITLE", required = true, value_parser = word)]
    words: Vec<String>,
}

fn word(s: &str) -> Result<String, &'static str> {
    if s.trim().is_empty() {
        Err("title can't be blank")
    } else {
        Ok(s.to_owned())
    }
}

fn resource_value(value: &str) -> Result<String, &'static str> {
    if value.trim().is_empty() {
        Err("value can't be blank")
    } else if value.trim() != value {
        Err("value can't start or end with whitespace")
    } else if value.chars().any(char::is_control) {
        Err("value can't contain control characters")
    } else {
        Ok(value.to_owned())
    }
}

fn resource_selector(resource: String, id: bool, owner: Option<String>) -> ResourceSelector {
    if id {
        ResourceSelector::Id(resource)
    } else {
        ResourceSelector::Name {
            name: resource,
            owner,
        }
    }
}

fn days_from_today(now: &Zoned, count: i32) -> Result<Range<Zoned>, jiff::Error> {
    let end = now.date().checked_add(count.days())?;
    Ok(now.start_of_day()?..end.to_zoned(now.time_zone().clone())?)
}

impl TaskOpts {
    fn add(self, words: &[String], now: &Zoned, config: &Config) -> Result<Action, ActionError> {
        let inline = if config.task.parse_inline {
            parse_inline(words)?
        } else {
            InlineTask {
                title: words.join(" "),
                list: None,
                due: None,
                priority: None,
            }
        };
        let due = self.due.or(inline.due);
        let (due, due_time) = due
            .map(|due| due.on(now.date()))
            .transpose()?
            .map_or((None, None), |(date, time)| (Some(date), time));
        let due_time_zone = due.zip(due_time).map(|(date, time)| {
            crate::domain::local_time_zone_name_at(now.time_zone(), date, time)
        });
        Ok(Action::AddTask(NewReminder {
            title: inline.title,
            list: self
                .list
                .or(inline.list)
                .or_else(|| config.task.list.clone()),
            due,
            due_time,
            due_time_zone,
            priority: self.priority.or(inline.priority).or(config.task.priority),
        }))
    }
}

#[derive(Debug, Error)]
pub(crate) enum ActionError {
    #[error(transparent)]
    Date(#[from] jiff::Error),
    #[error(transparent)]
    Inline(#[from] task::InlineError),
    #[error("--folder can't be used with `nib note folders`")]
    NoteFoldersFilter,
    #[error("--week, --calendar, and --show-calendar can't be used with `nib cal calendars`")]
    CalendarResourceFilter,
    #[error("--filter can only be used with `nib task` or `nib task list`")]
    TaskFilter,
    #[error("--filter can't be specified both before and after `list`")]
    DuplicateTaskFilter,
}

fn note_action(
    action: Option<NoteAction>,
    folder: Option<String>,
    config: &Config,
) -> Result<Action, ActionError> {
    Ok(match action {
        Some(NoteAction::New(Title { words })) => Action::NewNote(NewNote {
            title: words.join(" "),
            folder: folder.or_else(|| config.note.folder.clone()),
        }),
        Some(NoteAction::List) | None => Action::ListNotes(NoteQuery { folder }),
        Some(NoteAction::Folders(NoteFoldersArgs { action })) => {
            if folder.is_some() {
                return Err(ActionError::NoteFoldersFilter);
            }
            match action {
                None => Action::ListNoteFolders,
                Some(NoteFolderAction::Create { name, account }) => {
                    Action::CreateNoteFolder(NewResource {
                        name,
                        owner: account,
                    })
                }
                Some(NoteFolderAction::Rename {
                    resource,
                    new_name,
                    id,
                    account,
                }) => Action::RenameNoteFolder {
                    folder: resource_selector(resource, id, account),
                    name: new_name,
                },
            }
        }
    })
}

fn task_action(
    action: Option<TaskAction>,
    filter: Option<String>,
    now: &Zoned,
    config: &Config,
) -> Result<Action, ActionError> {
    if filter.is_some() && !matches!(action, Some(TaskAction::List { .. }) | None) {
        return Err(ActionError::TaskFilter);
    }
    Ok(match action {
        Some(TaskAction::Add {
            title: Title { words },
            opts,
        }) => opts.add(&words, now, config)?,
        Some(TaskAction::List {
            filter: list_filter,
        }) => match (filter, list_filter) {
            (Some(_), Some(_)) => return Err(ActionError::DuplicateTaskFilter),
            (filter, list_filter) => filter
                .or(list_filter)
                .map_or(Action::ListTasks, Action::FilterTasks),
        },
        None => filter.map_or(Action::ListTasks, Action::FilterTasks),
        Some(TaskAction::Lists(ReminderListsArgs { action })) => match action {
            None => Action::ListReminderLists,
            Some(ReminderListAction::Create { name, source }) => {
                Action::CreateReminderList(NewResource {
                    name,
                    owner: source,
                })
            }
            Some(ReminderListAction::Rename {
                resource,
                new_name,
                id,
                source,
            }) => Action::RenameReminderList {
                list: resource_selector(resource, id, source),
                name: new_name,
            },
        },
        Some(TaskAction::Edit { list }) => Action::EditTasks {
            list: list.or_else(|| config.task.list.clone()),
        },
        Some(TaskAction::Done(targets)) => quick::done(targets),
        Some(TaskAction::Reopen(targets)) => quick::reopen(targets),
        Some(TaskAction::Priority { level, targets }) => quick::priority(level, targets),
        Some(TaskAction::Move {
            destination,
            targets,
        }) => quick::move_to(destination, targets),
    })
}

fn calendar_action(args: CalArgs, now: &Zoned, config: &Config) -> Result<Action, ActionError> {
    let CalArgs {
        action,
        week,
        calendar,
        show_calendar,
    } = args;
    Ok(match action {
        Some(CalAction::List) | None => Action::ListEvents {
            query: EventQuery {
                range: days_from_today(now, if week { 7 } else { 1 })?,
                calendars: if calendar.is_empty() {
                    config.cal.calendars.clone()
                } else {
                    calendar
                },
            },
            show_calendar: show_calendar || config.cal.show_calendar,
        },
        Some(CalAction::Calendars(CalendarsArgs { action })) => {
            if week || !calendar.is_empty() || show_calendar {
                return Err(ActionError::CalendarResourceFilter);
            }
            match action {
                None => Action::ListCalendars,
                Some(CalendarResourceAction::Create { name, source }) => {
                    Action::CreateCalendar(NewResource {
                        name,
                        owner: source,
                    })
                }
                Some(CalendarResourceAction::Rename {
                    resource,
                    new_name,
                    id,
                    source,
                }) => Action::RenameCalendar {
                    calendar: resource_selector(resource, id, source),
                    name: new_name,
                },
            }
        }
    })
}

impl Command {
    pub(crate) fn into_action(
        self,
        now: &Zoned,
        config: &Config,
    ) -> Result<Option<Action>, ActionError> {
        Ok(Some(match self {
            Command::Complete { .. } | Command::Completions { .. } => return Ok(None),
            Command::Doctor => Action::Doctor,
            Command::Config(ConfigArgs { action }) => Action::Config(action),
            Command::Today(TodayArgs {
                calendar,
                show_calendar,
            }) => Action::Today {
                date: now.date(),
                query: EventQuery {
                    range: days_from_today(now, 1)?,
                    calendars: if calendar.is_empty() {
                        config.cal.calendars.clone()
                    } else {
                        calendar
                    },
                },
                show_calendar: show_calendar || config.cal.show_calendar,
            },
            Command::Note(NoteArgs {
                action,
                folder: Folder { folder },
            }) => note_action(action, folder, config)?,
            Command::N(NoteCapture {
                words,
                folder: Folder { folder },
            }) => {
                if words.is_empty() {
                    Action::ListNotes(NoteQuery { folder })
                } else {
                    Action::NewNote(NewNote {
                        title: words.join(" "),
                        folder: folder.or_else(|| config.note.folder.clone()),
                    })
                }
            }
            Command::Task(TaskArgs { action, filter }) => task_action(action, filter, now, config)?,
            Command::T(TaskCapture { words, opts }) => {
                if words.is_empty() {
                    Action::ListTasks
                } else {
                    opts.add(&words, now, config)?
                }
            }
            Command::Cal(args) => calendar_action(args, now, config)?,
            Command::Log => Action::Log,
            Command::Undo(UndoArgs { entry, resolve }) => Action::Undo { entry, resolve },
        }))
    }
}

impl Cli {
    pub fn is_doctor(&self) -> bool {
        matches!(self.command, Command::Doctor)
    }

    pub(crate) fn completion_request(&self) -> Option<CompletionRequest> {
        match self.command {
            Command::Complete { refresh, kind } => {
                Some(CompletionRequest::Values { kind, refresh })
            }
            Command::Completions { shell } => Some(CompletionRequest::Script(shell)),
            _ => None,
        }
    }

    pub fn is_config(&self) -> bool {
        matches!(self.command, Command::Config(_))
    }

    pub(crate) fn into_config(self) -> Option<(bool, ConfigAction)> {
        match self.command {
            Command::Config(ConfigArgs { action }) => Some((self.json, action)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionRequest {
    Script(CompletionShell),
    Values { kind: CompletionKind, refresh: bool },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use insta::assert_snapshot;
    use jiff::civil::{Date, date};
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::test_support::named_fixed_zone;

    fn at(day: Date, hour: i8, minute: i8) -> Zoned {
        day.at(hour, minute, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn parse_at(now: &Zoned, args: &[&str]) -> Result<Action, clap::Error> {
        Cli::try_parse_from(["nib"].iter().chain(args)).map(|cli| {
            cli.command
                .into_action(now, &Config::default())
                .unwrap()
                .unwrap()
        })
    }

    fn parse(args: &[&str]) -> Result<Action, clap::Error> {
        parse_at(&at(date(2026, 10, 26), 9, 30), args)
    }

    fn action_with_config(
        now: &Zoned,
        args: &[&str],
        config: &Config,
    ) -> Result<Action, ActionError> {
        Cli::try_parse_from(["nib"].iter().chain(args))
            .unwrap()
            .command
            .into_action(now, config)
            .map(|action| action.unwrap())
    }

    fn assert_parses(cases: &[(&[&str], Action)]) {
        for (args, expected) in cases {
            assert_eq!(&parse(args).unwrap(), expected, "nib {}", args.join(" "));
        }
    }

    fn note(title: &str, folder: Option<&str>) -> Action {
        Action::NewNote(NewNote {
            title: title.into(),
            folder: folder.map(Into::into),
        })
    }

    fn notes(folder: Option<&str>) -> Action {
        Action::ListNotes(NoteQuery {
            folder: folder.map(Into::into),
        })
    }

    fn task(title: &str, due: Option<Date>, priority: Option<Priority>) -> Action {
        Action::AddTask(NewReminder {
            title: title.into(),
            list: None,
            due,
            due_time: None,
            due_time_zone: None,
            priority,
        })
    }

    fn events(from: Date, until: Date) -> Action {
        Action::ListEvents {
            query: EventQuery {
                range: at(from, 0, 0)..at(until, 0, 0),
                calendars: Vec::new(),
            },
            show_calendar: false,
        }
    }

    fn today(calendars: &[&str]) -> Action {
        Action::Today {
            date: date(2026, 10, 26),
            query: EventQuery {
                range: at(date(2026, 10, 26), 0, 0)..at(date(2026, 10, 27), 0, 0),
                calendars: calendars.iter().map(|name| (*name).into()).collect(),
            },
            show_calendar: false,
        }
    }

    fn showing_calendar(action: Action) -> Action {
        match action {
            Action::ListEvents { query, .. } => Action::ListEvents {
                query,
                show_calendar: true,
            },
            Action::Today { date, query, .. } => Action::Today {
                date,
                query,
                show_calendar: true,
            },
            other => panic!("{other:?} doesn't list events"),
        }
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn today_command() {
        assert_parses(&[
            (&["today"], today(&[])),
            (
                &["today", "--calendar", "Work", "--calendar", "Personal"],
                today(&["Work", "Personal"]),
            ),
            (&["--json", "today"], today(&[])),
            (&["today", "--show-calendar"], showing_calendar(today(&[]))),
        ]);
    }

    #[test]
    fn doctor_command() {
        assert_eq!(parse(&["doctor"]).unwrap(), Action::Doctor);
    }

    #[test]
    fn config_commands() {
        assert_parses(&[
            (
                &["config", "list", "--origin"],
                Action::Config(ConfigAction::List { origin: true }),
            ),
            (
                &["config", "get", "task.list"],
                Action::Config(ConfigAction::Get {
                    key: "task.list".into(),
                }),
            ),
            (
                &["config", "set", "task.list", "Work", "--local"],
                Action::Config(ConfigAction::Set {
                    key: "task.list".into(),
                    value: "Work".into(),
                    target: ConfigTargetArgs {
                        local: true,
                        project: false,
                    },
                }),
            ),
            (
                &["config", "unset", "note.folder", "--project"],
                Action::Config(ConfigAction::Unset {
                    key: "note.folder".into(),
                    target: ConfigTargetArgs {
                        local: false,
                        project: true,
                    },
                }),
            ),
            (&["config", "path"], Action::Config(ConfigAction::Path)),
            (
                &["config", "get", "future.key"],
                Action::Config(ConfigAction::Get {
                    key: "future.key".into(),
                }),
            ),
        ]);

        let error = Cli::try_parse_from([
            "nib",
            "config",
            "set",
            "task.list",
            "Work",
            "--local",
            "--project",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn note_commands() {
        assert_parses(&[
            (&["n", "Quick", "thought"], note("Quick thought", None)),
            (&["n", "list", "-f", "Work"], note("list", Some("Work"))),
            (&["n"], notes(None)),
            (&["n", "-f", "Work"], notes(Some("Work"))),
            (&["note"], notes(None)),
            (&["note", "-f", "Work", "ls"], notes(Some("Work"))),
            (&["note", "list", "--folder", "Work"], notes(Some("Work"))),
            (&["note", "--json", "list"], notes(None)),
            (&["note", "folders"], Action::ListNoteFolders),
            (&["--json", "note", "folders"], Action::ListNoteFolders),
            (
                &[
                    "note",
                    "folders",
                    "create",
                    "Archive",
                    "--account",
                    "iCloud",
                ],
                Action::CreateNoteFolder(NewResource {
                    name: "Archive".into(),
                    owner: Some("iCloud".into()),
                }),
            ),
            (
                &[
                    "note",
                    "folders",
                    "rename",
                    "Work",
                    "Projects",
                    "--account",
                    "iCloud",
                ],
                Action::RenameNoteFolder {
                    folder: ResourceSelector::Name {
                        name: "Work".into(),
                        owner: Some("iCloud".into()),
                    },
                    name: "Projects".into(),
                },
            ),
            (
                &["note", "new", "Meeting with Team", "-f", "Work"],
                note("Meeting with Team", Some("Work")),
            ),
            (&["note", "new", "list"], note("list", None)),
        ]);
    }

    #[test]
    fn task_commands() {
        let review = task("Review PR", Some(date(2026, 10, 26)), Some(Priority::High));
        assert_parses(&[
            (
                &["t", "Review", "PR", "--due", "today", "-p", "high"],
                review.clone(),
            ),
            (
                &[
                    "todo",
                    "add",
                    "Review PR",
                    "-d",
                    "today",
                    "--priority",
                    "high",
                ],
                review,
            ),
            (&["t", "ls"], task("ls", None, None)),
            (&["t"], Action::ListTasks),
            (&["task"], Action::ListTasks),
            (&["todo", "ls"], Action::ListTasks),
            (&["task", "--json", "list"], Action::ListTasks),
            (
                &["task", "--filter", "due:today"],
                Action::FilterTasks("due:today".into()),
            ),
            (
                &["task", "list", "-f", "pri:high"],
                Action::FilterTasks("pri:high".into()),
            ),
            (&["task", "lists"], Action::ListReminderLists),
            (&["--json", "todo", "lists"], Action::ListReminderLists),
            (
                &["task", "lists", "create", "Errands"],
                Action::CreateReminderList(NewResource {
                    name: "Errands".into(),
                    owner: None,
                }),
            ),
            (
                &["task", "lists", "rename", "list-work", "Projects", "--id"],
                Action::RenameReminderList {
                    list: ResourceSelector::Id("list-work".into()),
                    name: "Projects".into(),
                },
            ),
        ]);
    }

    #[test]
    fn journal_commands() {
        assert_parses(&[
            (&["log"], Action::Log),
            (&["--json", "log"], Action::Log),
            (
                &["undo"],
                Action::Undo {
                    entry: None,
                    resolve: None,
                },
            ),
            (
                &["undo", "20261026T090000-journal-abcd"],
                Action::Undo {
                    entry: Some("20261026T090000-journal-abcd".into()),
                    resolve: None,
                },
            ),
            (
                &["undo", "--resolve", "20261026T090100-journal-efgh"],
                Action::Undo {
                    entry: None,
                    resolve: Some("20261026T090100-journal-efgh".into()),
                },
            ),
        ]);
    }

    #[test]
    fn cal_commands() {
        let day = events(date(2026, 10, 26), date(2026, 10, 27));
        let week = events(date(2026, 10, 26), date(2026, 11, 2));
        assert_parses(&[
            (&["c"], day.clone()),
            (&["cal", "--json", "list"], day.clone()),
            (&["cal", "--week"], week.clone()),
            (&["c", "-w", "ls"], week.clone()),
            (&["cal", "list", "-w"], week.clone()),
            (&["cal", "--show-calendar"], showing_calendar(day.clone())),
            (
                &["c", "ls", "--show-calendar", "-w"],
                showing_calendar(week),
            ),
            (&["cal", "calendars"], Action::ListCalendars),
            (&["--json", "c", "calendars"], Action::ListCalendars),
            (
                &["cal", "calendars", "create", "Travel", "--source", "iCloud"],
                Action::CreateCalendar(NewResource {
                    name: "Travel".into(),
                    owner: Some("iCloud".into()),
                }),
            ),
            (
                &[
                    "cal",
                    "calendars",
                    "rename",
                    "Work",
                    "Projects",
                    "--source",
                    "iCloud",
                ],
                Action::RenameCalendar {
                    calendar: ResourceSelector::Name {
                        name: "Work".into(),
                        owner: Some("iCloud".into()),
                    },
                    name: "Projects".into(),
                },
            ),
        ]);
    }

    #[test]
    fn resource_ids_conflict_with_owner_disambiguation() {
        for args in [
            &[
                "note",
                "folders",
                "rename",
                "folder-1",
                "Archive",
                "--id",
                "--account",
                "iCloud",
            ][..],
            &[
                "task", "lists", "rename", "list-1", "Archive", "--id", "--source", "iCloud",
            ][..],
            &[
                "cal",
                "calendars",
                "rename",
                "calendar-1",
                "Archive",
                "--id",
                "--source",
                "iCloud",
            ][..],
        ] {
            assert_eq!(
                Cli::try_parse_from(["nib"].into_iter().chain(args.iter().copied()))
                    .unwrap_err()
                    .kind(),
                ErrorKind::ArgumentConflict
            );
        }
    }

    #[test]
    fn resource_commands_reject_item_filters() {
        let now = at(date(2026, 10, 26), 9, 30);
        let config = Config::default();
        assert!(matches!(
            action_with_config(&now, &["note", "folders", "--folder", "Work"], &config),
            Err(ActionError::NoteFoldersFilter)
        ));
        let error =
            action_with_config(&now, &["task", "--filter", "work", "lists"], &config).unwrap_err();
        assert_eq!(
            error.to_string(),
            "--filter can only be used with `nib task` or `nib task list`"
        );
        assert!(matches!(
            action_with_config(
                &now,
                &["task", "--filter", "work", "list", "--filter", "home"],
                &config
            ),
            Err(ActionError::DuplicateTaskFilter)
        ));
        for args in [
            &["cal", "calendars", "--week"][..],
            &["cal", "calendars", "--calendar", "Work"][..],
            &["cal", "calendars", "--show-calendar"][..],
        ] {
            assert!(matches!(
                action_with_config(&now, args, &config),
                Err(ActionError::CalendarResourceFilter)
            ));
        }
    }

    #[test]
    fn configured_show_calendar_does_not_reject_calendar_resources() {
        let now = at(date(2026, 10, 26), 9, 30);
        let config = Config {
            cal: crate::domain::CalendarConfig {
                calendars: Vec::new(),
                show_calendar: true,
            },
            ..Config::default()
        };
        assert!(matches!(
            action_with_config(&now, &["cal", "calendars"], &config),
            Ok(Action::ListCalendars)
        ));
    }

    #[test]
    fn cal_ranges_follow_local_calendar_days() {
        let new_york = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let cases = [
            ("c", date(2026, 10, 26).at(23, 30, 0, 0)),
            ("c", date(2026, 11, 1).at(12, 0, 0, 0)),
            ("c", date(2027, 3, 14).at(12, 0, 0, 0)),
            ("c -w", date(2026, 10, 30).at(12, 0, 0, 0)),
        ];
        let ranges: Vec<String> = cases
            .into_iter()
            .map(|(args, now)| {
                let now = now.to_zoned(new_york.clone()).unwrap();
                let args: Vec<&str> = args.split(' ').collect();
                let Action::ListEvents {
                    query: EventQuery { range, .. },
                    ..
                } = parse_at(&now, &args).unwrap()
                else {
                    panic!("nib {args:?} should list events");
                };
                let time = "%Y-%m-%d %H:%M %:z";
                format!(
                    "{} at {}: {} .. {} ({}h)",
                    args.join(" "),
                    now.strftime(time),
                    range.start.strftime(time),
                    range.end.strftime(time),
                    range.start.duration_until(&range.end).as_hours(),
                )
            })
            .collect();
        assert_snapshot!(ranges.join("\n"), @"
        c at 2026-10-26 23:30 -04:00: 2026-10-26 00:00 -04:00 .. 2026-10-27 00:00 -04:00 (24h)
        c at 2026-11-01 12:00 -05:00: 2026-11-01 00:00 -04:00 .. 2026-11-02 00:00 -05:00 (25h)
        c at 2027-03-14 12:00 -04:00: 2027-03-14 00:00 -05:00 .. 2027-03-15 00:00 -04:00 (23h)
        c -w at 2026-10-30 12:00 -04:00: 2026-10-30 00:00 -04:00 .. 2026-11-06 00:00 -05:00 (169h)
        ");
    }

    #[test]
    fn blank_titles_are_rejected() {
        for args in [&["n", ""][..], &["t", "  "], &["note", "new", ""]] {
            let err = parse(args).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "{args:?}");
        }
    }

    #[test]
    fn blank_or_padded_resource_values_are_rejected() {
        for args in [
            &["note", "folders", "create", ""][..],
            &["task", "lists", "create", " Work"],
            &["cal", "calendars", "rename", "Work", "Projects "],
            &["task", "lists", "create", "Work", "--source", "  "],
            &["note", "folders", "rename", "Work", "Projects\nArchive"],
        ] {
            let err = parse(args).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "{args:?}");
        }
    }

    #[test]
    fn due_resolves_against_now() {
        let morning = at(date(2026, 10, 26), 9, 30);
        let late = at(date(2026, 10, 26), 23, 30);
        let new_years_eve = at(date(2026, 12, 31), 12, 0);
        let cases = [
            (&morning, "today", date(2026, 10, 26)),
            (&late, "today", date(2026, 10, 26)),
            (&late, "tomorrow", date(2026, 10, 27)),
            (&new_years_eve, "tomorrow", date(2027, 1, 1)),
            (&morning, "fri", date(2026, 10, 30)),
            (&morning, "friday", date(2026, 10, 30)),
            (&morning, "next fri", date(2026, 11, 6)),
            (&morning, "in 3d", date(2026, 10, 29)),
            (&morning, "in 2w", date(2026, 11, 9)),
            (&morning, "+2w", date(2026, 11, 9)),
            (&morning, "eom", date(2026, 10, 31)),
            (&morning, "2026-11-03", date(2026, 11, 3)),
            (&morning, "2025-01-31", date(2025, 1, 31)),
        ];
        for (now, due, expected) in cases {
            assert_eq!(
                parse_at(now, &["t", "Pay", "rent", "--due", due]).unwrap(),
                task("Pay rent", Some(expected), None),
                "--due {due} at {now}"
            );
        }
    }

    #[test]
    fn due_must_be_a_valid_date_or_time() {
        let errors: Vec<String> = [
            "banana",
            "",
            "2026-1-5",
            "25:00",
            "2026-11-03T25:00",
            "2026-02-30",
        ]
        .into_iter()
        .map(|due| {
            let err = parse(&["t", "Pay", "rent", "--due", due]).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "--due {due:?}");
            err.to_string().lines().next().unwrap().to_owned()
        })
        .collect();
        assert_snapshot!(errors.join("\n"), @"
        error: invalid value 'banana' for '--due <DUE>': expected today, tomorrow, a weekday, next WEEKDAY, in Nd, in Nw, +Nd, +Nw, eom, YYYY-MM-DD, a time, or a date followed by a time
        error: invalid value '' for '--due <DUE>': expected today, tomorrow, a weekday, next WEEKDAY, in Nd, in Nw, +Nd, +Nw, eom, YYYY-MM-DD, a time, or a date followed by a time
        error: invalid value '2026-1-5' for '--due <DUE>': expected YYYY-MM-DD
        error: invalid value '25:00' for '--due <DUE>': that time doesn't exist
        error: invalid value '2026-11-03T25:00' for '--due <DUE>': that time doesn't exist
        error: invalid value '2026-02-30' for '--due <DUE>': that date doesn't exist
        ");
    }

    #[test]
    fn inline_quick_add_works_for_both_task_commands() {
        let now = at(date(2026, 10, 26), 9, 30);
        let cases = [
            &["t", "Pay", "rent", "fri", "!high", "#bills"][..],
            &["task", "add", "Pay", "rent", "fri", "!high", "#bills"][..],
        ];
        for args in cases {
            assert_eq!(
                action_with_config(&now, args, &Config::default()).unwrap(),
                Action::AddTask(NewReminder {
                    title: "Pay rent".into(),
                    list: Some("bills".into()),
                    due: Some(date(2026, 10, 30)),
                    due_time: None,
                    due_time_zone: None,
                    priority: Some(Priority::High),
                }),
                "nib {}",
                args.join(" ")
            );
        }
    }

    #[test]
    fn every_due_form_works_inline() {
        let now = at(date(2026, 10, 26), 9, 30);
        let cases = [
            (&["t", "Do", "it", "today"][..], date(2026, 10, 26)),
            (&["t", "Do", "it", "tomorrow"][..], date(2026, 10, 27)),
            (&["t", "Do", "it", "fri"][..], date(2026, 10, 30)),
            (&["t", "Do", "it", "friday"][..], date(2026, 10, 30)),
            (&["t", "Do", "it", "next", "fri"][..], date(2026, 11, 6)),
            (&["t", "Do", "it", "in", "3d"][..], date(2026, 10, 29)),
            (&["t", "Do", "it", "in", "2w"][..], date(2026, 11, 9)),
            (&["t", "Do", "it", "+2w"][..], date(2026, 11, 9)),
            (&["t", "Do", "it", "eom"][..], date(2026, 10, 31)),
            (&["t", "Do", "it", "2027-01-05"][..], date(2027, 1, 5)),
        ];
        for (args, expected) in cases {
            assert_eq!(
                action_with_config(&now, args, &Config::default()).unwrap(),
                task("Do it", Some(expected), None),
                "nib {}",
                args.join(" ")
            );
        }
    }

    #[test]
    fn timed_quick_add_pins_to_the_local_zone() {
        let named_now = date(2026, 10, 26)
            .at(9, 30, 0, 0)
            .to_zoned(named_fixed_zone("America/Chicago", -6 * 60 * 60, *b"CST\0"))
            .unwrap();
        let named = action_with_config(
            &named_now,
            &["t", "Call", "team", "9am"],
            &Config::default(),
        )
        .unwrap();
        assert!(matches!(
            named,
            Action::AddTask(NewReminder { due_time_zone: Some(zone), .. })
                if zone == "America/Chicago"
        ));

        let posix_now = date(2026, 10, 26)
            .at(9, 30, 0, 0)
            .to_zoned(TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap())
            .unwrap();
        let fixed = action_with_config(
            &posix_now,
            &["t", "Call", "team", "9am"],
            &Config::default(),
        )
        .unwrap();
        assert!(matches!(
            fixed,
            Action::AddTask(NewReminder { due_time_zone: Some(zone), .. })
                if zone == "GMT-0500"
        ));

        let winter = action_with_config(
            &posix_now,
            &["t", "Call", "2026-12-01", "9am"],
            &Config::default(),
        )
        .unwrap();
        assert!(matches!(
            winter,
            Action::AddTask(NewReminder { due_time_zone: Some(zone), .. })
                if zone == "GMT-0600"
        ));

        let gap = action_with_config(
            &posix_now,
            &["t", "Call", "2027-03-14", "2:30am"],
            &Config::default(),
        )
        .unwrap();
        assert!(matches!(
            gap,
            Action::AddTask(NewReminder { due_time_zone: Some(zone), .. })
                if zone == "GMT-0600"
        ));
    }

    #[test]
    fn explicit_flags_override_inline_fields() {
        let now = at(date(2026, 10, 26), 9, 30);
        let cases = [
            &[
                "t",
                "Pay",
                "rent",
                "fri",
                "!low",
                "#bills",
                "--due",
                "tomorrow",
                "--priority",
                "high",
                "--list",
                "home",
            ][..],
            &[
                "task",
                "add",
                "Pay",
                "rent",
                "fri",
                "!low",
                "#bills",
                "--due",
                "tomorrow",
                "--priority",
                "high",
                "--list",
                "home",
            ][..],
        ];
        for args in cases {
            assert_eq!(
                action_with_config(&now, args, &Config::default()).unwrap(),
                Action::AddTask(NewReminder {
                    title: "Pay rent".into(),
                    list: Some("home".into()),
                    due: Some(date(2026, 10, 27)),
                    due_time: None,
                    due_time_zone: None,
                    priority: Some(Priority::High),
                }),
                "nib {}",
                args.join(" ")
            );
        }
    }

    #[test]
    fn configuration_can_disable_inline_parsing() {
        let config = Config {
            task: crate::domain::TaskConfig {
                parse_inline: false,
                ..crate::domain::TaskConfig::default()
            },
            ..Config::default()
        };
        let action = action_with_config(
            &at(date(2026, 10, 26), 9, 30),
            &["t", "Pay", "rent", "\\fri", "!high", "#bills"],
            &config,
        )
        .unwrap();
        assert_eq!(
            action,
            Action::AddTask(NewReminder {
                title: "Pay rent \\fri !high #bills".into(),
                list: None,
                due: None,
                due_time: None,
                due_time_zone: None,
                priority: None,
            })
        );
    }

    #[test]
    fn misplaced_input_is_rejected() {
        let cases: &[(&[&str], ErrorKind)] = &[
            (&["note", "new"], ErrorKind::MissingRequiredArgument),
            (&["t", "--due", "today"], ErrorKind::MissingRequiredArgument),
            (&["task", "--due", "today"], ErrorKind::UnknownArgument),
            (&["note", "Meeting"], ErrorKind::InvalidSubcommand),
        ];
        for (args, kind) in cases {
            assert_eq!(parse(args).unwrap_err().kind(), *kind, "{args:?}");
        }
    }

    #[test]
    fn configuration_defaults_apply_only_where_intended() {
        let config = Config {
            task: crate::domain::TaskConfig {
                list: Some("Snowglobe".into()),
                priority: Some(Priority::Medium),
                parse_inline: true,
            },
            cal: crate::domain::CalendarConfig {
                calendars: vec!["Work".into()],
                show_calendar: false,
            },
            note: crate::domain::NoteConfig {
                folder: Some("Inbox".into()),
            },
            ..Config::default()
        };
        let now = at(date(2026, 10, 26), 9, 30);
        let action = |args: &[&str]| {
            Cli::try_parse_from(["nib"].iter().chain(args))
                .unwrap()
                .command
                .into_action(&now, &config)
                .unwrap()
                .unwrap()
        };

        assert_eq!(
            action(&["t", "Review", "PR"]),
            Action::AddTask(NewReminder {
                title: "Review PR".into(),
                list: Some("Snowglobe".into()),
                due: None,
                due_time: None,
                due_time_zone: None,
                priority: Some(Priority::Medium),
            })
        );
        assert_eq!(
            action(&["task", "edit"]),
            Action::EditTasks {
                list: Some("Snowglobe".into())
            }
        );
        assert_eq!(action(&["n", "Idea"]), note("Idea", Some("Inbox")));
        assert_eq!(action(&["n"]), notes(None));
        assert!(matches!(
            action(&["cal"]),
            Action::ListEvents { query: EventQuery { calendars, .. }, show_calendar: false }
                if calendars == ["Work"]
        ));
        assert!(matches!(
            action(&["today"]),
            Action::Today { query: EventQuery { calendars, .. }, .. } if calendars == ["Work"]
        ));
        assert!(matches!(
            action(&["today", "--calendar", "Personal"]),
            Action::Today { query: EventQuery { calendars, .. }, .. } if calendars == ["Personal"]
        ));
    }

    #[test]
    fn show_calendar_comes_from_the_flag_or_configuration() {
        let now = at(date(2026, 10, 26), 9, 30);
        let shown = |args: &[&str], configured: bool| {
            let config = Config {
                cal: crate::domain::CalendarConfig {
                    calendars: Vec::new(),
                    show_calendar: configured,
                },
                ..Config::default()
            };
            match action_with_config(&now, args, &config).unwrap() {
                Action::ListEvents { show_calendar, .. } | Action::Today { show_calendar, .. } => {
                    show_calendar
                }
                other => panic!("{other:?} doesn't list events"),
            }
        };

        for args in [&["cal"][..], &["today"][..]] {
            assert!(!shown(args, false), "{args:?}");
            assert!(shown(args, true), "{args:?}");
        }
        for args in [
            &["cal", "--show-calendar"][..],
            &["today", "--show-calendar"][..],
        ] {
            assert!(shown(args, false), "{args:?}");
            assert!(shown(args, true), "{args:?}");
        }
    }
}

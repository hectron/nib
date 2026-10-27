use std::ops::Range;

use clap::{Args, Parser, Subcommand};
use jiff::civil::Date;
use jiff::{ToSpan, Zoned};

use crate::backend::{EventQuery, NewNote, NewReminder, NoteQuery, Priority};
use crate::config::Config;

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
    NewNote(NewNote),
    ListNotes(NoteQuery),
    AddTask(NewReminder),
    ListTasks,
    EditTasks { list: Option<String> },
    ListEvents(EventQuery),
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create and list notes
    Note(NoteArgs),
    /// Add and list reminders
    #[command(visible_alias = "todo")]
    Task(TaskArgs),
    /// Show calendar events
    #[command(visible_alias = "c")]
    Cal(CalArgs),
    /// Create a note from the rest of the line, or list notes
    N(NoteCapture),
    /// Add a reminder from the rest of the line, or list reminders
    T(TaskCapture),
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
    List,
    /// Edit reminders as a Markdown file in $VISUAL or $EDITOR
    Edit {
        /// Only edit this list
        #[arg(short, long)]
        list: Option<String>,
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
    /// Due date: today, tomorrow, or YYYY-MM-DD
    #[arg(short, long, value_name = "DATE", value_parser = due, requires = "words")]
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
}

#[derive(Debug, Subcommand)]
enum CalAction {
    /// List events (the default)
    #[command(visible_alias = "ls")]
    List,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Due {
    Today,
    Tomorrow,
    On(Date),
}

fn due(s: &str) -> Result<Due, &'static str> {
    match s {
        "today" => Ok(Due::Today),
        "tomorrow" => Ok(Due::Tomorrow),
        _ if is_iso_date(s) => s
            .parse()
            .map(Due::On)
            .map_err(|_| "that date doesn't exist"),
        _ => Err("expected today, tomorrow, or YYYY-MM-DD"),
    }
}

fn is_iso_date(s: &str) -> bool {
    s.len() == 10
        && s.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

impl Due {
    fn on(self, today: Date) -> Result<Date, jiff::Error> {
        match self {
            Self::Today => Ok(today),
            Self::Tomorrow => today.tomorrow(),
            Self::On(date) => Ok(date),
        }
    }
}

fn days_from_today(now: &Zoned, count: i32) -> Result<Range<Zoned>, jiff::Error> {
    let end = now.date().checked_add(count.days())?;
    Ok(now.start_of_day()?..end.to_zoned(now.time_zone().clone())?)
}

impl TaskOpts {
    fn add(self, title: String, now: &Zoned, config: &Config) -> Result<Action, jiff::Error> {
        Ok(Action::AddTask(NewReminder {
            title,
            list: self.list.or_else(|| config.task.list.clone()),
            due: self.due.map(|due| due.on(now.date())).transpose()?,
            priority: self.priority.or(config.task.priority),
        }))
    }
}

impl Command {
    pub fn into_action(self, now: &Zoned, config: &Config) -> Result<Action, jiff::Error> {
        Ok(match self {
            Command::Note(NoteArgs {
                action,
                folder: Folder { folder },
            }) => match action {
                Some(NoteAction::New(Title { words })) => Action::NewNote(NewNote {
                    title: words.join(" "),
                    folder: folder.or_else(|| config.note.folder.clone()),
                }),
                Some(NoteAction::List) | None => Action::ListNotes(NoteQuery { folder }),
            },
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
            Command::Task(TaskArgs { action }) => match action {
                Some(TaskAction::Add {
                    title: Title { words },
                    opts,
                }) => opts.add(words.join(" "), now, config)?,
                Some(TaskAction::List) | None => Action::ListTasks,
                Some(TaskAction::Edit { list }) => Action::EditTasks {
                    list: list.or_else(|| config.task.list.clone()),
                },
            },
            Command::T(TaskCapture { words, opts }) => {
                if words.is_empty() {
                    Action::ListTasks
                } else {
                    opts.add(words.join(" "), now, config)?
                }
            }
            Command::Cal(CalArgs {
                action,
                week,
                calendar,
            }) => match action {
                Some(CalAction::List) | None => Action::ListEvents(EventQuery {
                    range: days_from_today(now, if week { 7 } else { 1 })?,
                    calendars: if calendar.is_empty() {
                        config.cal.calendars.clone()
                    } else {
                        calendar
                    },
                }),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use insta::assert_snapshot;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;

    fn at(day: Date, hour: i8, minute: i8) -> Zoned {
        day.at(hour, minute, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn parse_at(now: &Zoned, args: &[&str]) -> Result<Action, clap::Error> {
        Cli::try_parse_from(["nib"].iter().chain(args))
            .map(|cli| cli.command.into_action(now, &Config::default()).unwrap())
    }

    fn parse(args: &[&str]) -> Result<Action, clap::Error> {
        parse_at(&at(date(2026, 10, 26), 9, 30), args)
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
            priority,
        })
    }

    fn events(from: Date, until: Date) -> Action {
        Action::ListEvents(EventQuery {
            range: at(from, 0, 0)..at(until, 0, 0),
            calendars: Vec::new(),
        })
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
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
        ]);
    }

    #[test]
    fn cal_commands() {
        let day = events(date(2026, 10, 26), date(2026, 10, 27));
        let week = events(date(2026, 10, 26), date(2026, 11, 2));
        assert_parses(&[
            (&["c"], day.clone()),
            (&["cal", "--json", "list"], day),
            (&["cal", "--week"], week.clone()),
            (&["c", "-w", "ls"], week.clone()),
            (&["cal", "list", "-w"], week),
        ]);
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
                let Action::ListEvents(EventQuery { range, .. }) = parse_at(&now, &args).unwrap()
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
    fn due_resolves_against_now() {
        let morning = at(date(2026, 10, 26), 9, 30);
        let late = at(date(2026, 10, 26), 23, 30);
        let new_years_eve = at(date(2026, 12, 31), 12, 0);
        let cases = [
            (&morning, "today", date(2026, 10, 26)),
            (&late, "today", date(2026, 10, 26)),
            (&late, "tomorrow", date(2026, 10, 27)),
            (&new_years_eve, "tomorrow", date(2027, 1, 1)),
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
    fn due_must_be_a_date() {
        let errors: Vec<String> = ["banana", "", "2026-1-5", "2026-11-03T10:00", "2026-02-30"]
            .into_iter()
            .map(|due| {
                let err = parse(&["t", "Pay", "rent", "--due", due]).unwrap_err();
                assert_eq!(err.kind(), ErrorKind::ValueValidation, "--due {due:?}");
                err.to_string().lines().next().unwrap().to_owned()
            })
            .collect();
        assert_snapshot!(errors.join("\n"), @"
        error: invalid value 'banana' for '--due <DATE>': expected today, tomorrow, or YYYY-MM-DD
        error: invalid value '' for '--due <DATE>': expected today, tomorrow, or YYYY-MM-DD
        error: invalid value '2026-1-5' for '--due <DATE>': expected today, tomorrow, or YYYY-MM-DD
        error: invalid value '2026-11-03T10:00' for '--due <DATE>': expected today, tomorrow, or YYYY-MM-DD
        error: invalid value '2026-02-30' for '--due <DATE>': that date doesn't exist
        ");
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
            task: crate::config::Task {
                list: Some("Snowglobe".into()),
                priority: Some(Priority::Medium),
            },
            cal: crate::config::Cal {
                calendars: vec!["Work".into()],
            },
            note: crate::config::Note {
                folder: Some("Inbox".into()),
            },
        };
        let now = at(date(2026, 10, 26), 9, 30);
        let action = |args: &[&str]| {
            Cli::try_parse_from(["nib"].iter().chain(args))
                .unwrap()
                .command
                .into_action(&now, &config)
                .unwrap()
        };

        assert_eq!(
            action(&["t", "Review", "PR"]),
            Action::AddTask(NewReminder {
                title: "Review PR".into(),
                list: Some("Snowglobe".into()),
                due: None,
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
            Action::ListEvents(EventQuery { calendars, .. }) if calendars == ["Work"]
        ));
    }
}

use clap::{Args, Parser, Subcommand};

use crate::backend::{EventQuery, NewNote, NewReminder, NoteQuery, Priority, Span};

/// Fast CLI for Apple Notes, Reminders, and Calendar
#[derive(Debug, Parser)]
#[command(name = "jot", version)]
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
    /// Due date, such as today or 2026-11-03
    #[arg(short, long, requires = "words")]
    due: Option<String>,
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

impl TaskOpts {
    fn add(self, title: String) -> Action {
        Action::AddTask(NewReminder {
            title,
            due: self.due,
            priority: self.priority,
        })
    }
}

impl Command {
    pub fn into_action(self) -> Action {
        match self {
            Command::Note(NoteArgs {
                action,
                folder: Folder { folder },
            }) => match action {
                Some(NoteAction::New(Title { words })) => Action::NewNote(NewNote {
                    title: words.join(" "),
                    folder,
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
                        folder,
                    })
                }
            }
            Command::Task(TaskArgs { action }) => match action {
                Some(TaskAction::Add {
                    title: Title { words },
                    opts,
                }) => opts.add(words.join(" ")),
                Some(TaskAction::List) | None => Action::ListTasks,
            },
            Command::T(TaskCapture { words, opts }) => {
                if words.is_empty() {
                    Action::ListTasks
                } else {
                    opts.add(words.join(" "))
                }
            }
            Command::Cal(CalArgs { action, week }) => match action {
                Some(CalAction::List) | None => Action::ListEvents(EventQuery {
                    span: if week { Span::Week } else { Span::Day },
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;
    use insta::assert_snapshot;

    use super::*;

    fn parse(args: &[&str]) -> Result<Action, clap::Error> {
        Cli::try_parse_from(["jot"].iter().chain(args)).map(|cli| cli.command.into_action())
    }

    fn assert_parses(cases: &[(&[&str], Action)]) {
        for (args, expected) in cases {
            assert_eq!(&parse(args).unwrap(), expected, "jot {}", args.join(" "));
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

    fn task(title: &str, due: Option<&str>, priority: Option<Priority>) -> Action {
        Action::AddTask(NewReminder {
            title: title.into(),
            due: due.map(Into::into),
            priority,
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
        let review = task("Review PR", Some("today"), Some(Priority::High));
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
        let day = || Action::ListEvents(EventQuery { span: Span::Day });
        let week = || Action::ListEvents(EventQuery { span: Span::Week });
        assert_parses(&[
            (&["c"], day()),
            (&["cal", "--json", "list"], day()),
            (&["cal", "--week"], week()),
            (&["c", "-w", "ls"], week()),
            (&["cal", "list", "-w"], week()),
        ]);
    }

    #[test]
    fn blank_titles_are_rejected() {
        for args in [&["n", ""][..], &["t", "  "], &["note", "new", ""]] {
            let err = parse(args).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ValueValidation, "{args:?}");
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
}

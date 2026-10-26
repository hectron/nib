use clap::{Args, Parser, Subcommand};

use crate::backend::{Priority, Span};

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

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Capture and list notes
    #[command(visible_alias = "n")]
    Note(NoteArgs),
    /// Add and list reminders
    #[command(visible_aliases = ["t", "todo"])]
    Task(TaskArgs),
    /// Show calendar events
    #[command(visible_alias = "c")]
    Cal(CalArgs),
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct NoteArgs {
    #[command(subcommand)]
    action: Option<NoteAction>,
    /// Create a note with this title (same as `jot note new`)
    title: Option<String>,
    #[command(flatten)]
    opts: NoteOpts,
}

#[derive(Debug, PartialEq, Subcommand)]
pub enum NoteAction {
    /// Create a note
    New {
        title: String,
        #[command(flatten)]
        opts: NoteOpts,
    },
    /// List notes
    #[command(visible_alias = "ls")]
    List {
        #[command(flatten)]
        opts: NoteOpts,
    },
}

#[derive(Debug, Default, PartialEq, Args)]
pub struct NoteOpts {
    /// Folder to create in or list from
    #[arg(short, long)]
    pub folder: Option<String>,
}

impl NoteArgs {
    pub fn into_action(self) -> NoteAction {
        match (self.action, self.title) {
            (Some(action), _) => action,
            (None, Some(title)) => NoteAction::New {
                title,
                opts: self.opts,
            },
            (None, None) => NoteAction::List { opts: self.opts },
        }
    }
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct TaskArgs {
    #[command(subcommand)]
    action: Option<TaskAction>,
    /// Add a reminder with this title (same as `jot task add`)
    title: Option<String>,
    #[command(flatten)]
    opts: TaskOpts,
}

#[derive(Debug, PartialEq, Subcommand)]
pub enum TaskAction {
    /// Add a reminder
    Add {
        title: String,
        #[command(flatten)]
        opts: TaskOpts,
    },
    /// List reminders
    #[command(visible_alias = "ls")]
    List,
}

#[derive(Debug, Default, PartialEq, Args)]
pub struct TaskOpts {
    /// Due date, such as today or 2026-11-03
    #[arg(short, long, requires = "title")]
    pub due: Option<String>,
    #[arg(short, long, value_enum, requires = "title")]
    pub priority: Option<Priority>,
}

impl TaskArgs {
    pub fn into_action(self) -> TaskAction {
        match (self.action, self.title) {
            (Some(action), _) => action,
            (None, Some(title)) => TaskAction::Add {
                title,
                opts: self.opts,
            },
            (None, None) => TaskAction::List,
        }
    }
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct CalArgs {
    #[command(subcommand)]
    action: Option<CalAction>,
    #[command(flatten)]
    opts: CalOpts,
}

#[derive(Debug, PartialEq, Subcommand)]
pub enum CalAction {
    /// List events (the default)
    #[command(visible_alias = "ls")]
    List {
        #[command(flatten)]
        opts: CalOpts,
    },
}

#[derive(Debug, Default, PartialEq, Args)]
pub struct CalOpts {
    /// Show the next seven days instead of today
    #[arg(short, long)]
    week: bool,
}

impl CalOpts {
    pub fn span(&self) -> Span {
        if self.week { Span::Week } else { Span::Day }
    }
}

impl CalArgs {
    pub fn into_action(self) -> CalAction {
        self.action.unwrap_or(CalAction::List { opts: self.opts })
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        Cli::try_parse_from(["jot"].iter().chain(args)).map(|cli| cli.command)
    }

    fn note(args: &[&str]) -> NoteAction {
        match parse(args).unwrap() {
            Command::Note(a) => a.into_action(),
            other => panic!("expected note, got {other:?}"),
        }
    }

    fn task(args: &[&str]) -> TaskAction {
        match parse(args).unwrap() {
            Command::Task(a) => a.into_action(),
            other => panic!("expected task, got {other:?}"),
        }
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn bare_title_creates_note() {
        let expected = NoteAction::New {
            title: "Quick thought".into(),
            opts: NoteOpts {
                folder: Some("Work".into()),
            },
        };
        assert_eq!(note(&["n", "Quick thought", "-f", "Work"]), expected);
        assert_eq!(
            note(&["note", "new", "Quick thought", "--folder", "Work"]),
            expected
        );
    }

    #[test]
    fn subcommand_name_wins_over_title() {
        let list = NoteAction::List {
            opts: NoteOpts::default(),
        };
        assert_eq!(note(&["note", "list"]), list);
        assert_eq!(note(&["note"]), list);
    }

    #[test]
    fn bare_title_adds_task() {
        let expected = TaskAction::Add {
            title: "Review PR".into(),
            opts: TaskOpts {
                due: Some("today".into()),
                priority: Some(Priority::High),
            },
        };
        assert_eq!(
            task(&["t", "Review PR", "--due", "today", "-p", "high"]),
            expected
        );
        assert_eq!(
            task(&[
                "todo",
                "add",
                "Review PR",
                "-d",
                "today",
                "--priority",
                "high"
            ]),
            expected
        );
        assert_eq!(task(&["task"]), TaskAction::List);
    }

    #[test]
    fn task_options_need_a_title() {
        let err = parse(&["task", "--due", "today"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn cal_defaults_to_today() {
        let Command::Cal(args) = parse(&["c"]).unwrap() else {
            panic!("expected cal");
        };
        let CalAction::List { opts } = args.into_action();
        assert_eq!(opts.span(), Span::Day);

        let Command::Cal(args) = parse(&["cal", "--week"]).unwrap() else {
            panic!("expected cal");
        };
        let CalAction::List { opts } = args.into_action();
        assert_eq!(opts.span(), Span::Week);
    }
}

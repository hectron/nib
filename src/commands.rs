use miette::Result;

use crate::backend::Backends;
use crate::cli::{Action, Cli};
use crate::output::{self, Style};

pub fn run(cli: Cli, backends: &Backends, style: Style) -> Result<String> {
    let json = cli.json;
    match cli.command.into_action() {
        Action::NewNote(note) => output::single(&backends.notes.create(note)?, json),
        Action::ListNotes(query) => output::list(&backends.notes.list(query)?, json, style),
        Action::AddTask(reminder) => output::single(&backends.reminders.add(reminder)?, json),
        Action::ListTasks => output::list(&backends.reminders.list()?, json, style),
        Action::ListEvents(query) => output::list(&backends.calendar.events(query)?, json, style),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use insta::assert_snapshot;

    use super::*;
    use crate::backend::{
        BackendError, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery, Notes,
        Priority, Reminder, Reminders, Span,
    };

    struct Fake;

    impl Notes for Fake {
        fn create(&self, note: NewNote) -> Result<Note, BackendError> {
            Ok(Note {
                title: note.title,
                folder: note.folder,
            })
        }

        fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError> {
            let notes = [
                ("Groceries", "Personal"),
                ("Q4 planning", "Work"),
                ("Standup", "Work"),
            ];
            Ok(notes
                .into_iter()
                .filter(|(_, f)| query.folder.as_deref().is_none_or(|want| want == *f))
                .map(|(title, f)| Note {
                    title: title.into(),
                    folder: Some(f.into()),
                })
                .collect())
        }
    }

    impl Reminders for Fake {
        fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
            Ok(Reminder {
                title: reminder.title,
                due: reminder.due,
                priority: reminder.priority,
            })
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            Ok(vec![
                Reminder {
                    title: "Review pull request".into(),
                    due: Some("2026-10-27".into()),
                    priority: Some(Priority::High),
                },
                Reminder {
                    title: "Buy groceries".into(),
                    due: None,
                    priority: None,
                },
            ])
        }
    }

    impl Calendar for Fake {
        fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
            let mut events = vec![Event {
                title: "Team sync".into(),
                start: "2026-10-26 10:00".into(),
            }];
            if query.span == Span::Week {
                events.push(Event {
                    title: "Design review".into(),
                    start: "2026-10-29 14:30".into(),
                });
            }
            Ok(events)
        }
    }

    fn jot(args: &[&str]) -> String {
        let backends = Backends {
            notes: Box::new(Fake),
            reminders: Box::new(Fake),
            calendar: Box::new(Fake),
        };
        let cli = Cli::try_parse_from(["jot"].iter().chain(args)).unwrap();
        run(cli, &backends, Style::default()).unwrap()
    }

    #[test]
    fn quick_capture_note() {
        assert_snapshot!(jot(&["n", "Quick thought"]), @r#"Created note "Quick thought""#);
        assert_snapshot!(
            jot(&["note", "new", "Meeting with Team", "--folder", "Work"]),
            @r#"Created note "Meeting with Team" in Work"#
        );
    }

    #[test]
    fn note_list_filters_by_folder() {
        assert_snapshot!(jot(&["note", "ls", "-f", "Work"]), @r"
        TITLE        FOLDER
        Q4 planning  Work
        Standup      Work
        ");
        assert_snapshot!(jot(&["note", "list", "-f", "Archive"]), @"");
    }

    #[test]
    fn quick_capture_task() {
        assert_snapshot!(jot(&["t", "Buy groceries"]), @r#"Added "Buy groceries""#);
        assert_snapshot!(
            jot(&["task", "add", "Review pull request", "--due", "today", "--priority", "high"]),
            @r#"Added "Review pull request" (due today, high priority)"#
        );
    }

    #[test]
    fn task_list_table() {
        assert_snapshot!(jot(&["task"]), @r"
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-27  high
        Buy groceries
        ");
    }

    #[test]
    fn task_list_json() {
        assert_snapshot!(jot(&["todo", "list", "--json"]), @r#"
        [
          {
            "title": "Review pull request",
            "due": "2026-10-27",
            "priority": "high"
          },
          {
            "title": "Buy groceries",
            "due": null,
            "priority": null
          }
        ]
        "#);
    }

    #[test]
    fn created_item_json_is_an_object() {
        assert_snapshot!(jot(&["--json", "n", "Idea", "-f", "Inbox"]), @r#"
        {
          "title": "Idea",
          "folder": "Inbox"
        }
        "#);
    }

    #[test]
    fn cal_spans() {
        assert_snapshot!(jot(&["cal"]), @r"
        START             TITLE
        2026-10-26 10:00  Team sync
        ");
        assert_snapshot!(jot(&["c", "list", "--week"]), @r"
        START             TITLE
        2026-10-26 10:00  Team sync
        2026-10-29 14:30  Design review
        ");
    }

    #[test]
    fn options_before_subcommand() {
        assert_snapshot!(jot(&["note", "-f", "Work", "list"]), @r"
        TITLE        FOLDER
        Q4 planning  Work
        Standup      Work
        ");
        assert_eq!(
            jot(&["task", "--json", "ls"]),
            jot(&["task", "ls", "--json"])
        );
        assert_eq!(
            jot(&["cal", "--json", "list"]),
            jot(&["cal", "list", "--json"])
        );
        assert_eq!(jot(&["c", "-w", "ls"]), jot(&["c", "ls", "-w"]));
    }

    #[test]
    fn capture_takes_the_rest_of_the_line() {
        assert_snapshot!(jot(&["n", "Buy", "oat", "milk"]), @r#"Created note "Buy oat milk""#);
        assert_snapshot!(jot(&["n", "list"]), @r#"Created note "list""#);
        assert_snapshot!(jot(&["note", "new", "list"]), @r#"Created note "list""#);
        assert_snapshot!(jot(&["t", "ls", "-p", "low"]), @r#"Added "ls" (low priority)"#);
    }
}

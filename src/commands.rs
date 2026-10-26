use jiff::Zoned;
use miette::{IntoDiagnostic, Result};

use crate::backend::Backends;
use crate::cli::{Action, Cli};
use crate::output::{self, Style};

pub fn run(cli: Cli, now: &Zoned, backends: &Backends, style: Style) -> Result<String> {
    let json = cli.json;
    match cli.command.into_action(now).into_diagnostic()? {
        Action::NewNote(note) => {
            output::single(&backends.notes.create(note)?, "Created note", json)
        }
        Action::ListNotes(query) => output::list(&backends.notes.list(query)?, json, style),
        Action::AddTask(reminder) => {
            output::single(&backends.reminders.add(reminder)?, "Added", json)
        }
        Action::ListTasks => output::list(&backends.reminders.list()?, json, style),
        Action::ListEvents(query) => output::list(&backends.calendar.events(query)?, json, style),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use clap::Parser;
    use insta::assert_snapshot;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::backend::{
        BackendError, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery, Notes,
        Priority, Reminder, Reminders, Span,
    };

    struct Fake;

    impl Notes for Fake {
        fn create(&self, note: NewNote) -> Result<Note, BackendError> {
            Ok(Note {
                id: "note-4".into(),
                title: note.title,
                folder: note.folder,
            })
        }

        fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError> {
            let notes = [
                ("note-1", "Groceries", "Personal"),
                ("note-2", "Q4 planning", "Work"),
                ("note-3", "Standup", "Work"),
            ];
            Ok(notes
                .into_iter()
                .filter(|(_, _, f)| query.folder.as_deref().is_none_or(|want| want == *f))
                .map(|(id, title, f)| Note {
                    id: id.into(),
                    title: title.into(),
                    folder: Some(f.into()),
                })
                .collect())
        }
    }

    impl Reminders for Fake {
        fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
            Ok(Reminder {
                id: "reminder-3".into(),
                title: reminder.title,
                due: reminder.due,
                priority: reminder.priority,
            })
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            Ok(vec![
                Reminder {
                    id: "reminder-1".into(),
                    title: "Review pull request".into(),
                    due: Some(date(2026, 10, 27)),
                    priority: Some(Priority::High),
                },
                Reminder {
                    id: "reminder-2".into(),
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
                id: "event-1".into(),
                title: "Team sync".into(),
                start: "2026-10-26 10:00".into(),
            }];
            if query.span == Span::Week {
                events.push(Event {
                    id: "event-2".into(),
                    title: "Design review".into(),
                    start: "2026-10-29 14:30".into(),
                });
            }
            Ok(events)
        }
    }

    type Built = Rc<RefCell<Vec<&'static str>>>;

    fn fake_backends(built: &Built) -> Backends {
        let (notes, reminders, calendar) = (built.clone(), built.clone(), built.clone());
        Backends::new(
            move || {
                notes.borrow_mut().push("notes");
                Box::new(Fake)
            },
            move || {
                reminders.borrow_mut().push("reminders");
                Box::new(Fake)
            },
            move || {
                calendar.borrow_mut().push("calendar");
                Box::new(Fake)
            },
        )
    }

    fn now() -> Zoned {
        date(2026, 10, 26)
            .at(9, 30, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn jot(args: &[&str]) -> String {
        let backends = fake_backends(&Built::default());
        let cli = Cli::try_parse_from(["jot"].iter().chain(args)).unwrap();
        run(cli, &now(), &backends, Style::default()).unwrap()
    }

    fn built_by(args: &[&str]) -> Vec<&'static str> {
        let built = Built::default();
        let backends = fake_backends(&built);
        let cli = Cli::try_parse_from(["jot"].iter().chain(args)).unwrap();
        run(cli, &now(), &backends, Style::default()).unwrap();
        built.take()
    }

    #[test]
    fn each_command_builds_only_its_backend() {
        assert_eq!(built_by(&["n", "Quick thought"]), ["notes"]);
        assert_eq!(built_by(&["note", "ls", "-f", "Work"]), ["notes"]);
        assert_eq!(built_by(&["t", "Buy groceries"]), ["reminders"]);
        assert_eq!(built_by(&["task"]), ["reminders"]);
        assert_eq!(built_by(&["cal", "--week"]), ["calendar"]);
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
    fn note_list_json() {
        assert_snapshot!(jot(&["note", "ls", "-f", "Work", "--json"]), @r#"
        [
          {
            "id": "note-2",
            "title": "Q4 planning",
            "folder": "Work"
          },
          {
            "id": "note-3",
            "title": "Standup",
            "folder": "Work"
          }
        ]
        "#);
    }

    #[test]
    fn quick_capture_task() {
        assert_snapshot!(jot(&["t", "Buy groceries"]), @r#"Added "Buy groceries""#);
        assert_snapshot!(
            jot(&["task", "add", "Review pull request", "--due", "today", "--priority", "high"]),
            @r#"Added "Review pull request" (due 2026-10-26, high priority)"#
        );
        assert_snapshot!(
            jot(&["t", "Pay", "rent", "-d", "tomorrow"]),
            @r#"Added "Pay rent" (due 2026-10-27)"#
        );
    }

    #[test]
    fn added_task_json() {
        assert_snapshot!(
            jot(&["--json", "t", "Review pull request", "-d", "2026-11-03", "-p", "high"]),
            @r#"
        {
          "id": "reminder-3",
          "title": "Review pull request",
          "due": "2026-11-03",
          "priority": "high"
        }
        "#
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
            "id": "reminder-1",
            "title": "Review pull request",
            "due": "2026-10-27",
            "priority": "high"
          },
          {
            "id": "reminder-2",
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
          "id": "note-4",
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
    fn event_list_json() {
        assert_snapshot!(jot(&["cal", "--week", "--json"]), @r#"
        [
          {
            "id": "event-1",
            "title": "Team sync",
            "start": "2026-10-26T10:00:00-04:00"
          },
          {
            "id": "event-2",
            "title": "Design review",
            "start": "2026-10-29T14:30:00-04:00"
          }
        ]
        "#);
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

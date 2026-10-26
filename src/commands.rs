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
    use jiff::civil::{Date, date};
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::backend::{
        BackendError, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery, Notes,
        Priority, Reminder, Reminders,
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
            let events = [
                ("event-1", "Book club", at(date(2026, 10, 25), 19, 0)),
                ("event-2", "Team sync", at(date(2026, 10, 26), 10, 0)),
                ("event-3", "Design review", at(date(2026, 10, 29), 14, 30)),
                ("event-4", "Retro", at(date(2026, 11, 2), 9, 0)),
            ];
            Ok(events
                .into_iter()
                .filter(|(_, _, start)| query.range.contains(start))
                .map(|(id, title, start)| Event {
                    id: id.into(),
                    title: title.into(),
                    start,
                })
                .collect())
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

    fn at(day: Date, hour: i8, minute: i8) -> Zoned {
        day.at(hour, minute, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn now() -> Zoned {
        at(date(2026, 10, 26), 9, 30)
    }

    fn nib(args: &[&str]) -> String {
        let backends = fake_backends(&Built::default());
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        run(cli, &now(), &backends, Style::default()).unwrap()
    }

    fn built_by(args: &[&str]) -> Vec<&'static str> {
        let built = Built::default();
        let backends = fake_backends(&built);
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
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
        assert_snapshot!(nib(&["n", "Quick thought"]), @r#"Created note "Quick thought""#);
        assert_snapshot!(
            nib(&["note", "new", "Meeting with Team", "--folder", "Work"]),
            @r#"Created note "Meeting with Team" in Work"#
        );
    }

    #[test]
    fn note_list_filters_by_folder() {
        assert_snapshot!(nib(&["note", "ls", "-f", "Work"]), @r"
        TITLE        FOLDER
        Q4 planning  Work
        Standup      Work
        ");
        assert_snapshot!(nib(&["note", "list", "-f", "Archive"]), @"");
    }

    #[test]
    fn note_list_json() {
        assert_snapshot!(nib(&["note", "ls", "-f", "Work", "--json"]), @r#"
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
        assert_snapshot!(nib(&["t", "Buy groceries"]), @r#"Added "Buy groceries""#);
        assert_snapshot!(
            nib(&["task", "add", "Review pull request", "--due", "today", "--priority", "high"]),
            @r#"Added "Review pull request" (due 2026-10-26, high priority)"#
        );
        assert_snapshot!(
            nib(&["t", "Pay", "rent", "-d", "tomorrow"]),
            @r#"Added "Pay rent" (due 2026-10-27)"#
        );
    }

    #[test]
    fn added_task_json() {
        assert_snapshot!(
            nib(&["--json", "t", "Review pull request", "-d", "2026-11-03", "-p", "high"]),
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
        assert_snapshot!(nib(&["task"]), @r"
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-27  high
        Buy groceries
        ");
    }

    #[test]
    fn task_list_json() {
        assert_snapshot!(nib(&["todo", "list", "--json"]), @r#"
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
        assert_snapshot!(nib(&["--json", "n", "Idea", "-f", "Inbox"]), @r#"
        {
          "id": "note-4",
          "title": "Idea",
          "folder": "Inbox"
        }
        "#);
    }

    #[test]
    fn cal_spans() {
        assert_snapshot!(nib(&["cal"]), @r"
        START             TITLE
        2026-10-26 10:00  Team sync
        ");
        assert_snapshot!(nib(&["c", "list", "--week"]), @r"
        START             TITLE
        2026-10-26 10:00  Team sync
        2026-10-29 14:30  Design review
        ");
    }

    #[test]
    fn event_list_json() {
        assert_snapshot!(nib(&["cal", "--week", "--json"]), @r#"
        [
          {
            "id": "event-2",
            "title": "Team sync",
            "start": "2026-10-26T10:00:00-04:00"
          },
          {
            "id": "event-3",
            "title": "Design review",
            "start": "2026-10-29T14:30:00-04:00"
          }
        ]
        "#);
    }

    #[test]
    fn options_before_subcommand() {
        assert_snapshot!(nib(&["note", "-f", "Work", "list"]), @r"
        TITLE        FOLDER
        Q4 planning  Work
        Standup      Work
        ");
        assert_eq!(
            nib(&["task", "--json", "ls"]),
            nib(&["task", "ls", "--json"])
        );
        assert_eq!(
            nib(&["cal", "--json", "list"]),
            nib(&["cal", "list", "--json"])
        );
        assert_eq!(nib(&["c", "-w", "ls"]), nib(&["c", "ls", "-w"]));
    }

    #[test]
    fn capture_takes_the_rest_of_the_line() {
        assert_snapshot!(nib(&["n", "Buy", "oat", "milk"]), @r#"Created note "Buy oat milk""#);
        assert_snapshot!(nib(&["n", "list"]), @r#"Created note "list""#);
        assert_snapshot!(nib(&["note", "new", "list"]), @r#"Created note "list""#);
        assert_snapshot!(nib(&["t", "ls", "-p", "low"]), @r#"Added "ls" (low priority)"#);
    }
}

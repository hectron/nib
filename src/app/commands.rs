use jiff::Zoned;
use miette::{IntoDiagnostic, Result};

use crate::app::edit;
use crate::cli::output::{self, Style};
use crate::cli::{Action, Cli};
use crate::domain::{Backends, Config};
use crate::infra::host::Host;

pub fn run(
    cli: Cli,
    config: &Config,
    now: &Zoned,
    backends: &Backends,
    style: Style,
    host: &mut dyn Host,
) -> Result<String> {
    let json = cli.json;
    match cli.command.into_action(now, config).into_diagnostic()? {
        Action::NewNote(note) => {
            output::single(&backends.notes.create(note)?, "Created note", json)
        }
        Action::ListNotes(query) => output::list(&backends.notes.list(query)?, json, style),
        Action::AddTask(reminder) => {
            output::single(&backends.reminders.add(reminder)?, "Added", json)
        }
        Action::ListTasks => output::list(&backends.reminders.list()?, json, style),
        Action::EditTasks { list } => edit::tasks(list.as_deref(), now, backends, host),
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
    use crate::domain::{
        BackendError, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery, Notes,
        Priority, Reminder, ReminderChange, ReminderSnapshot, Reminders,
    };
    use crate::infra::host::Scripted;

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
            if let Some(name) = reminder
                .list
                .as_deref()
                .filter(|name| !matches!(*name, "Reminders" | "Work"))
            {
                return Err(BackendError::NotFound {
                    kind: "writable reminder list",
                    name: name.into(),
                });
            }
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

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            Ok(ReminderSnapshot::default())
        }

        fn apply(&self, _: &[ReminderChange]) -> Result<(), BackendError> {
            Ok(())
        }
    }

    impl Calendar for Fake {
        fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
            let events = [
                (
                    "event-1",
                    "Book club",
                    at(date(2026, 10, 25), 19, 0),
                    false,
                    "Personal",
                ),
                (
                    "event-2",
                    "Team sync",
                    at(date(2026, 10, 26), 10, 0),
                    false,
                    "Work",
                ),
                (
                    "event-5",
                    "Offsite",
                    at(date(2026, 10, 28), 0, 0),
                    true,
                    "Work",
                ),
                (
                    "event-3",
                    "Design review",
                    at(date(2026, 10, 29), 14, 30),
                    false,
                    "Work",
                ),
                (
                    "event-4",
                    "Retro",
                    at(date(2026, 11, 2), 9, 0),
                    false,
                    "Work",
                ),
            ];
            for name in &query.calendars {
                if !events.iter().any(|event| event.4 == name) {
                    return Err(BackendError::NotFound {
                        kind: "calendar",
                        name: name.clone(),
                    });
                }
            }
            Ok(events
                .into_iter()
                .filter(|(_, _, start, _, calendar)| {
                    query.range.contains(start)
                        && (query.calendars.is_empty()
                            || query.calendars.iter().any(|name| name == calendar))
                })
                .map(|(id, title, start, all_day, _)| Event {
                    id: id.into(),
                    title: title.into(),
                    start,
                    all_day,
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
        run(
            cli,
            &Config::default(),
            &now(),
            &backends,
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap()
    }

    fn built_by(args: &[&str]) -> Vec<&'static str> {
        let built = Built::default();
        let backends = fake_backends(&built);
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        run(
            cli,
            &Config::default(),
            &now(),
            &backends,
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap();
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
        START               TITLE
        2026-10-26 10:00    Team sync
        2026-10-28 all day  Offsite
        2026-10-29 14:30    Design review
        ");
    }

    #[test]
    fn event_list_json() {
        assert_snapshot!(nib(&["cal", "--week", "--json"]), @r#"
        [
          {
            "id": "event-2",
            "title": "Team sync",
            "start": "2026-10-26T10:00:00-04:00",
            "all_day": false
          },
          {
            "id": "event-5",
            "title": "Offsite",
            "start": "2026-10-28T00:00:00-04:00",
            "all_day": true
          },
          {
            "id": "event-3",
            "title": "Design review",
            "start": "2026-10-29T14:30:00-04:00",
            "all_day": false
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

    #[test]
    fn task_configuration_reaches_the_backend() {
        struct Recorder(Rc<RefCell<Option<NewReminder>>>);

        impl Reminders for Recorder {
            fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
                self.0.replace(Some(reminder.clone()));
                Ok(Reminder {
                    id: "recorded".into(),
                    title: reminder.title,
                    due: reminder.due,
                    priority: reminder.priority,
                })
            }

            fn list(&self) -> Result<Vec<Reminder>, BackendError> {
                unreachable!()
            }

            fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
                unreachable!()
            }

            fn apply(&self, _: &[ReminderChange]) -> Result<(), BackendError> {
                unreachable!()
            }
        }

        let recorded = Rc::new(RefCell::new(None));
        let backend_recorded = recorded.clone();
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(Recorder(backend_recorded)),
            || -> Box<dyn Calendar> { unreachable!() },
        );
        let config = Config {
            task: crate::domain::TaskConfig {
                list: Some("Snowglobe".into()),
                priority: Some(Priority::Medium),
            },
            ..Config::default()
        };
        let cli = Cli::try_parse_from(["nib", "t", "Review", "PR"]).unwrap();
        run(
            cli,
            &config,
            &now(),
            &backends,
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap();

        assert_eq!(
            recorded.take(),
            Some(NewReminder {
                title: "Review PR".into(),
                list: Some("Snowglobe".into()),
                due: None,
                priority: Some(Priority::Medium),
            })
        );
    }

    #[test]
    fn calendar_configuration_reaches_the_backend() {
        struct Recorder(Rc<RefCell<Option<EventQuery>>>);

        impl Calendar for Recorder {
            fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
                self.0.replace(Some(query));
                Ok(Vec::new())
            }
        }

        let recorded = Rc::new(RefCell::new(None));
        let backend_recorded = recorded.clone();
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            || -> Box<dyn Reminders> { unreachable!() },
            move || Box::new(Recorder(backend_recorded)),
        );
        let config = Config {
            cal: crate::domain::CalendarConfig {
                calendars: vec!["Work".into(), "Birthdays".into()],
            },
            ..Config::default()
        };
        let cli = Cli::try_parse_from(["nib", "cal"]).unwrap();
        run(
            cli,
            &config,
            &now(),
            &backends,
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap();

        assert_eq!(recorded.take().unwrap().calendars, ["Work", "Birthdays"]);
    }
}

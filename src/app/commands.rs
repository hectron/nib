use jiff::Zoned;
use miette::{IntoDiagnostic, Result};

use crate::app::{edit, journal, undo};
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
        Action::Doctor => Err(miette::miette!(
            "doctor must be run through the system diagnostic entry point"
        )),
        Action::Today { date, query } => {
            let reminders = backends.reminders.list()?;
            let (overdue, remaining): (Vec<_>, Vec<_>) = reminders
                .into_iter()
                .partition(|reminder| reminder.due.is_some_and(|due| due < date));
            let due_today = remaining
                .into_iter()
                .filter(|reminder| reminder.due == Some(date))
                .collect::<Vec<_>>();
            let events = backends.calendar.events(query)?;
            output::today(&overdue, &due_today, &events, date, json, style)
        }
        Action::NewNote(note) => {
            output::single(&backends.notes.create(note)?, "Created note", json, style)
        }
        Action::ListNotes(query) => {
            output::list(&backends.notes.list(query)?, json, now.date(), style)
        }
        Action::AddTask(reminder) => {
            output::single(&backends.reminders.add(reminder)?, "Added", json, style)
        }
        Action::ListTasks => output::list(&backends.reminders.list()?, json, now.date(), style),
        Action::EditTasks { list } => edit::tasks(list.as_deref(), now, backends, host),
        Action::ListEvents(query) => {
            output::list(&backends.calendar.events(query)?, json, now.date(), style)
        }
        Action::Log => output::journal(
            &journal::summaries(now.timestamp(), host)?,
            json,
            now.date(),
            style,
        ),
        Action::Undo {
            entry: _,
            resolve: Some(id),
        } => {
            let resolution = match undo::resolve(&id, now, host)? {
                undo::Resolution::Applied => Some(true),
                undo::Resolution::Removed => Some(false),
                undo::Resolution::Cancelled => None,
            };
            output::undo_resolution(&id, resolution, json, style)
        }
        Action::Undo {
            entry,
            resolve: None,
        } => match undo::run(entry.as_deref(), now, backends, host)? {
            undo::Outcome::Cancelled { id } => output::undo_cancelled(&id, json, style),
            undo::Outcome::Applied {
                id,
                undo_id,
                counts,
            } => output::undo(&id, &undo_id, counts, json, style),
        },
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
        ApplyResult, AutomationStatus, BackendError, Calendar, ChangeCounts, DoctorProbe,
        EditorSetting, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery, Notes,
        PermissionStatus, Priority, Reminder, ReminderChange, ReminderSnapshot, Reminders,
    };
    use crate::infra::host::Scripted;

    struct Fake;

    struct FakeDoctor;

    impl DoctorProbe for FakeDoctor {
        fn macos_version(&self) -> std::result::Result<String, String> {
            Ok("26.1".into())
        }

        fn terminal(&self) -> (String, bool) {
            ("Ghostty".into(), true)
        }

        fn reminders_authorization(&self) -> PermissionStatus {
            PermissionStatus::FullAccess
        }

        fn calendars_authorization(&self) -> PermissionStatus {
            PermissionStatus::Denied
        }

        fn notes_automation(&self) -> AutomationStatus {
            AutomationStatus::TargetNotRunning
        }

        fn editor(&self) -> EditorSetting {
            EditorSetting {
                variable: Some("VISUAL"),
                command: Some("code".into()),
            }
        }
    }

    impl Notes for Fake {
        fn create(&self, note: NewNote) -> Result<Note, BackendError> {
            Ok(Note {
                id: "note-4".into(),
                title: note.title,
                folder: note.folder,
                modified: Some("2026-10-26T14:00:00Z".parse().unwrap()),
                locked: false,
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
                    modified: Some(
                        match id {
                            "note-1" => "2026-10-26T11:00:00Z",
                            "note-2" => "2026-10-26T13:00:00Z",
                            _ => "2026-10-26T12:00:00Z",
                        }
                        .parse()
                        .unwrap(),
                    ),
                    locked: id == "note-3",
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

        fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
            Ok(ApplyResult::default())
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

    struct AgendaReminders(Vec<Reminder>);

    impl Reminders for AgendaReminders {
        fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
            unreachable!()
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            Ok(self.0.clone())
        }

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            unreachable!()
        }

        fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
            unreachable!()
        }
    }

    struct AgendaCalendar(Vec<Event>);

    impl Calendar for AgendaCalendar {
        fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
            Ok(self
                .0
                .iter()
                .filter(|event| query.range.contains(&event.start))
                .cloned()
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

    fn agenda(args: &[&str], reminders: Vec<Reminder>, events: Vec<Event>) -> String {
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(AgendaReminders(reminders)),
            move || Box::new(AgendaCalendar(events)),
        );
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

    fn reminder(id: &str, title: &str, due: Option<Date>) -> Reminder {
        Reminder {
            id: id.into(),
            title: title.into(),
            due,
            priority: None,
        }
    }

    fn event(id: &str, title: &str, day: Date, hour: i8) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start: at(day, hour, 0),
            all_day: false,
        }
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
        assert_eq!(built_by(&["today"]), ["reminders", "calendar"]);
        assert_eq!(built_by(&["n", "Quick thought"]), ["notes"]);
        assert_eq!(built_by(&["note", "ls", "-f", "Work"]), ["notes"]);
        assert_eq!(built_by(&["t", "Buy groceries"]), ["reminders"]);
        assert_eq!(built_by(&["task"]), ["reminders"]);
        assert_eq!(built_by(&["cal", "--week"]), ["calendar"]);
        assert_eq!(built_by(&["log"]), Vec::<&str>::new());
    }

    fn journal_entry(time: &str, action: &str, undo_of: Option<&str>, change: &str) -> String {
        let reminder = serde_json::json!({
            "id": "reminder-1",
            "list_id": "list-1",
            "title": "Buy groceries",
            "notes": null,
            "due": null,
            "due_components": null,
            "priority": null,
            "completed": false,
            "modified": null,
            "alarms_present": false,
            "recurrence_present": false,
            "url": null,
            "partial": false
        });
        let journal_change = if change == "deleted" {
            serde_json::json!({ "change": change, "backup": reminder })
        } else {
            serde_json::json!({ "change": change, "written": reminder })
        };
        serde_json::json!({
            "version": 1,
            "time": time,
            "action": action,
            "undo_of": undo_of,
            "changes": [journal_change]
        })
        .to_string()
    }

    fn journal_output(args: &[&str]) -> String {
        let backends = fake_backends(&Built::default());
        let older = journal_entry("2026-10-25T14:00:00Z", "task_edit", None, "added");
        let newer = journal_entry(
            "2026-10-26T14:00:00Z",
            "undo",
            Some("older-journal"),
            "deleted",
        );
        let mut host = Scripted::new()
            .journal("older-journal", &older)
            .journal("newer-journal", &newer);
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        run(
            cli,
            &Config::default(),
            &now(),
            &backends,
            Style::default(),
            &mut host,
        )
        .unwrap()
    }

    #[test]
    fn log_lists_newest_entries_with_counts() {
        assert_snapshot!(journal_output(&["log"]), @r"
        ENTRY          WRITTEN               ACTION     STATUS   UNDO OF        UNDONE  ADDED  UPDATED  COMPLETED  DELETED
        newer-journal  2026-10-26T14:00:00Z  undo       applied  older-journal          0      0        0          1
        older-journal  2026-10-25T14:00:00Z  task_edit  applied                 yes     1      0        0          0
        ");
    }

    #[test]
    fn log_json_has_stable_fields() {
        assert_snapshot!(journal_output(&["log", "--json"]), @r#"
        [
          {
            "id": "newer-journal",
            "time": "2026-10-26T14:00:00Z",
            "written_at": "2026-10-26T14:00:00Z",
            "action": "undo",
            "status": "applied",
            "undo_of": "older-journal",
            "undone": false,
            "added": 0,
            "updated": 0,
            "completed": 0,
            "deleted": 1
          },
          {
            "id": "older-journal",
            "time": "2026-10-25T14:00:00Z",
            "written_at": "2026-10-25T14:00:00Z",
            "action": "task_edit",
            "status": "applied",
            "undo_of": null,
            "undone": true,
            "added": 1,
            "updated": 0,
            "completed": 0,
            "deleted": 0
          }
        ]
        "#);
    }

    #[test]
    fn undo_json_has_stable_fields() {
        assert_snapshot!(output::undo(
            "20261026T090000-journal-abcd",
            "20261026T090100-journal-efgh",
            ChangeCounts {
                added: 1,
                updated: 2,
                completed: 3,
                deleted: 4,
            },
            true,
            Style::default(),
        )
        .unwrap(), @r#"
        {
          "id": "20261026T090000-journal-abcd",
          "undo_id": "20261026T090100-journal-efgh",
          "applied": true,
          "added": 1,
          "updated": 2,
          "completed": 3,
          "deleted": 4
        }
        "#);
    }

    #[test]
    fn undo_resolution_json_has_stable_fields() {
        assert_snapshot!(output::undo_resolution(
            "20261026T090100-journal-efgh",
            Some(true),
            true,
            Style::default(),
        )
        .unwrap(), @r#"
        {
          "id": "20261026T090100-journal-efgh",
          "resolved": true,
          "applied": true
        }
        "#);
    }

    #[test]
    fn cancelled_undo_json_is_an_object() {
        assert_snapshot!(output::undo_cancelled(
            "20261026T090000-journal-abcd",
            true,
            Style::default(),
        )
        .unwrap(), @r#"
        {
          "id": "20261026T090000-journal-abcd",
          "undo_id": null,
          "applied": false,
          "added": 0,
          "updated": 0,
          "completed": 0,
          "deleted": 0
        }
        "#);
    }

    #[test]
    fn today_groups_each_agenda_section() {
        let reminders = vec![
            reminder(
                "reminder-overdue",
                "Submit expense report",
                Some(date(2026, 10, 24)),
            ),
            reminder(
                "reminder-today",
                "Review pull request",
                Some(date(2026, 10, 26)),
            ),
            reminder("reminder-future", "Renew passport", Some(date(2026, 11, 9))),
            reminder("reminder-undated", "Buy groceries", None),
        ];
        let events = vec![event(
            "event-today",
            "Design review",
            date(2026, 10, 26),
            14,
        )];
        assert_snapshot!(agenda(&["today"], reminders, events), @r"
        Overdue
        TITLE                  DUE         PRIORITY
        Submit expense report  2026-10-24

        Due today
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-26

        Events
        START             TITLE
        2026-10-26 14:00  Design review
        ");
    }

    #[test]
    fn today_omits_empty_sections() {
        let events = vec![event(
            "event-today",
            "Design review",
            date(2026, 10, 26),
            14,
        )];
        assert_snapshot!(agenda(&["today"], Vec::new(), events), @r"
        Events
        START             TITLE
        2026-10-26 14:00  Design review
        ");
        assert_snapshot!(agenda(&["today"], Vec::new(), Vec::new()), @"Nothing due today.");
    }

    #[test]
    fn today_json_has_one_array_per_section() {
        let reminders = vec![
            reminder(
                "reminder-overdue",
                "Submit expense report",
                Some(date(2026, 10, 24)),
            ),
            reminder(
                "reminder-today",
                "Review pull request",
                Some(date(2026, 10, 26)),
            ),
        ];
        let events = vec![event(
            "event-today",
            "Design review",
            date(2026, 10, 26),
            14,
        )];
        assert_snapshot!(agenda(&["today", "--json"], reminders, events), @r#"
        {
          "overdue": [
            {
              "id": "reminder-overdue",
              "title": "Submit expense report",
              "due": "2026-10-24",
              "priority": null
            }
          ],
          "due_today": [
            {
              "id": "reminder-today",
              "title": "Review pull request",
              "due": "2026-10-26",
              "priority": null
            }
          ],
          "events": [
            {
              "id": "event-today",
              "title": "Design review",
              "start": "2026-10-26T14:00:00-04:00",
              "all_day": false
            }
          ]
        }
        "#);
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
        TITLE        FOLDER  LOCKED
        Q4 planning  Work
        Standup      Work    yes
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
            "folder": "Work",
            "modified": "2026-10-26T13:00:00Z",
            "locked": false
          },
          {
            "id": "note-3",
            "title": "Standup",
            "folder": "Work",
            "modified": "2026-10-26T12:00:00Z",
            "locked": true
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
          "folder": "Inbox",
          "modified": "2026-10-26T14:00:00Z",
          "locked": false
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
        TITLE        FOLDER  LOCKED
        Q4 planning  Work
        Standup      Work    yes
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

            fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
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
                parse_inline: true,
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

    #[test]
    fn doctor_text_reports_checks_and_fixes() {
        let checks = crate::app::doctor::checks(
            &FakeDoctor,
            &[crate::app::doctor::ConfigFileCheck {
                path: "/Users/me/project/nib.toml".into(),
                error: None,
            }],
            None,
        );
        assert!(crate::app::doctor::failed(&checks));
        assert_snapshot!(output::doctor(&checks, false, Style::default()).unwrap(), @r###"
        ✓ macOS 26.1 is supported
        ✓ macOS attributes permissions to Ghostty
        ✓ Ghostty has full access to Reminders
        ✗ Ghostty access to Calendars is denied
          Fix: Turn on Ghostty in System Settings > Privacy & Security > Calendars.
        ! Apple Notes Automation access is checked on first use
          Fix: When prompted, allow Ghostty to control Notes; later changes are in System Settings > Privacy & Security > Automation.
        ✓ config is valid: /Users/me/project/nib.toml
        ! $VISUAL uses `code` without a wait flag
          Fix: Set VISUAL="code --wait".
        "###);
    }

    #[test]
    fn doctor_json_has_stable_check_objects() {
        let checks = crate::app::doctor::checks(
            &FakeDoctor,
            &[crate::app::doctor::ConfigFileCheck {
                path: "/Users/me/project/nib.toml".into(),
                error: None,
            }],
            None,
        );
        assert_snapshot!(output::doctor(&checks, true, Style::default()).unwrap(), @r###"
        {
          "id": "doctor",
          "ok": false,
          "checks": [
            {
              "id": "macos",
              "status": "pass",
              "message": "macOS 26.1 is supported",
              "fix": null
            },
            {
              "id": "terminal",
              "status": "pass",
              "message": "macOS attributes permissions to Ghostty",
              "fix": null
            },
            {
              "id": "reminders",
              "status": "pass",
              "message": "Ghostty has full access to Reminders",
              "fix": null
            },
            {
              "id": "calendars",
              "status": "fail",
              "message": "Ghostty access to Calendars is denied",
              "fix": "Turn on Ghostty in System Settings > Privacy & Security > Calendars."
            },
            {
              "id": "notes",
              "status": "warning",
              "message": "Apple Notes Automation access is checked on first use",
              "fix": "When prompted, allow Ghostty to control Notes; later changes are in System Settings > Privacy & Security > Automation."
            },
            {
              "id": "config_1",
              "status": "pass",
              "message": "config is valid: /Users/me/project/nib.toml",
              "fix": null
            },
            {
              "id": "editor",
              "status": "warning",
              "message": "$VISUAL uses `code` without a wait flag",
              "fix": "Set VISUAL=\"code --wait\"."
            }
          ]
        }
        "###);
    }
}

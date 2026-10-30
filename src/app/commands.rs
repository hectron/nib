use std::collections::HashMap;

use jiff::Zoned;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use miette::Result;

use crate::app::filter::{Filter, FilterKind, FilterRow};
use crate::app::{edit, journal, quick, undo};
use crate::cli::Action;
use crate::cli::output::{self, Style};
use crate::domain::{
    Backends, EventQuery, Reminder, ReminderDueComponents, ReminderSnapshot, StoredReminder,
    TaskChangeRequest,
};
use crate::infra::host::Host;

pub fn run(
    action: Action,
    json: bool,
    now: &Zoned,
    backends: &Backends,
    style: Style,
    host: &mut dyn Host,
) -> Result<String> {
    match action {
        Action::Doctor | Action::Config(_) | Action::Pick { .. } => Err(miette::miette!(
            "doctor, config, and pick must be run through their own entry points"
        )),
        Action::Today {
            date,
            query,
            show_calendar,
        } => today(date, query, show_calendar, json, now, backends, style),
        Action::NewNote(note) => {
            output::single(&backends.notes.create(note)?, "Created note", json, style)
        }
        Action::ListNotes(query) => output::list(&backends.notes.list(query)?, json, now, style),
        Action::ListNoteFolders => output::list(&backends.notes.folders()?, json, now, style),
        Action::CreateNoteFolder(folder) => output::single(
            &backends.notes.create_folder(folder)?,
            "Created folder",
            json,
            style,
        ),
        Action::RenameNoteFolder { folder, name } => output::single(
            &backends.notes.rename_folder(folder, name)?,
            "Renamed folder",
            json,
            style,
        ),
        Action::AddTask(reminder) => output::single_at(
            &backends.reminders.add(reminder)?,
            "Added",
            json,
            style,
            now,
        ),
        Action::ListTasks => output::list(&backends.reminders.list()?, json, now, style),
        Action::FilterTasks(expression) => filtered_tasks(&expression, json, now, backends, style),
        Action::ListReminderLists => output::list(&backends.reminders.lists()?, json, now, style),
        Action::CreateReminderList(list) => output::single(
            &backends.reminders.create_list(list)?,
            "Created list",
            json,
            style,
        ),
        Action::RenameReminderList { list, name } => output::single(
            &backends.reminders.rename_list(list, name)?,
            "Renamed list",
            json,
            style,
        ),
        Action::EditTasks { list } => edit::tasks(list.as_deref(), now, backends, host),
        Action::ChangeTasks(request) => change_tasks(&request, json, now, backends, style, host),
        Action::ListEvents {
            query,
            show_calendar,
        } => output::events(
            &backends.calendar.events(query)?,
            show_calendar,
            json,
            now,
            style,
        ),
        Action::ListCalendars => output::list(&backends.calendar.calendars()?, json, now, style),
        Action::CreateCalendar(calendar) => output::single(
            &backends.calendar.create_calendar(calendar)?,
            "Created calendar",
            json,
            style,
        ),
        Action::RenameCalendar { calendar, name } => output::single(
            &backends.calendar.rename_calendar(calendar, name)?,
            "Renamed calendar",
            json,
            style,
        ),
        Action::Log => output::journal(&journal::log(now.timestamp(), host)?, json, now, style),
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

fn change_tasks(
    request: &TaskChangeRequest,
    json: bool,
    now: &Zoned,
    backends: &Backends,
    style: Style,
    host: &mut dyn Host,
) -> Result<String> {
    output::changed_reminders(&quick::run(request, now, backends, host)?, json, style)
}

fn filtered_tasks(
    expression: &str,
    json: bool,
    now: &Zoned,
    backends: &Backends,
    style: Style,
) -> Result<String> {
    let filter = Filter::parse(expression)?;
    let snapshot = if filter.includes_completed() {
        backends.reminders.snapshot_including_completed()?
    } else {
        backends.reminders.snapshot(None)?
    };
    let ReminderSnapshot { lists, reminders } = snapshot;
    let list_names = lists
        .iter()
        .map(|list| (list.id.as_str(), list.name.as_str()))
        .collect::<HashMap<_, _>>();
    let reminders = reminders
        .into_iter()
        .filter_map(|stored| {
            let collection = list_names
                .get(stored.list_id.as_str())
                .copied()
                .unwrap_or_default();
            let completed = stored.completed;
            let reminder = reminder_from_snapshot(stored, now.time_zone());
            filter
                .matches(
                    &FilterRow {
                        kind: FilterKind::Reminder,
                        title: &reminder.title,
                        collection,
                        due: reminder.due,
                        due_time: reminder.due_time,
                        due_at: reminder.due_at,
                        priority: reminder.priority,
                        completed,
                    },
                    now,
                )
                .then_some(reminder)
        })
        .collect::<Vec<_>>();
    output::list(&reminders, json, now, style)
}

pub(crate) fn reminder_from_snapshot(reminder: StoredReminder, local_zone: &TimeZone) -> Reminder {
    let due_time = reminder.due_components.as_ref().and_then(stored_due_time);
    let due_time_zone = due_time.and_then(|_| {
        reminder
            .due_components
            .as_ref()
            .and_then(|parts| parts.time_zone.clone())
    });
    let due_at = reminder.due.zip(due_time).and_then(|(date, time)| {
        let zone = match due_time_zone.as_deref() {
            Some(name) => TimeZone::get(name).ok()?,
            None => local_zone.clone(),
        };
        date.to_datetime(time)
            .to_zoned(zone)
            .ok()
            .map(|due| due.timestamp())
    });
    Reminder {
        id: reminder.id,
        title: reminder.title,
        due: reminder.due,
        due_time,
        due_time_zone,
        due_at,
        priority: reminder.priority,
    }
}

fn stored_due_time(parts: &ReminderDueComponents) -> Option<Time> {
    Time::new(
        parts.hour?.try_into().ok()?,
        parts.minute?.try_into().ok()?,
        0,
        0,
    )
    .ok()
}

fn today(
    date: Date,
    query: EventQuery,
    show_calendar: bool,
    json: bool,
    now: &Zoned,
    backends: &Backends,
    style: Style,
) -> Result<String> {
    let reminders = backends.reminders.list()?;
    let (overdue, remaining): (Vec<_>, Vec<_>) = reminders
        .into_iter()
        .partition(|reminder| output::reminder_is_overdue(reminder, now));
    let due_today = remaining
        .into_iter()
        .filter(|reminder| output::reminder_is_due_today(reminder, date, now))
        .collect::<Vec<_>>();
    let events = backends.calendar.events(query)?;
    output::today(
        &overdue,
        &due_today,
        &events,
        show_calendar,
        now,
        json,
        style,
    )
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
        ApplyResult, AutomationStatus, BackendError, Calendar, CalendarInfo, CalendarKind,
        ChangeCounts, ConfigEntry, ConfigKey, ConfigOrigin, ConfigValue, DoctorProbe,
        EditorSetting, Event, EventQuery, NewNote, NewReminder, NewResource, Note, NoteFolder,
        NoteQuery, Notes, PermissionStatus, Priority, Reminder, ReminderChange, ReminderList,
        ReminderListInfo, ReminderSnapshot, Reminders, ResourceSelector, StoredReminder,
    };
    use crate::infra::host::Scripted;
    use crate::{Cli, Config};

    struct Fake;

    fn selector_id(selector: &ResourceSelector, fallback: &str) -> String {
        match selector {
            ResourceSelector::Id(id) => id.clone(),
            ResourceSelector::Name { .. } => fallback.into(),
        }
    }

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

        fn folders(&self) -> Result<Vec<NoteFolder>, BackendError> {
            Ok(vec![
                NoteFolder {
                    id: "folder-personal".into(),
                    name: "Personal".into(),
                    account: Some("iCloud".into()),
                    account_id: Some("account-icloud".into()),
                },
                NoteFolder {
                    id: "folder-work".into(),
                    name: "Work".into(),
                    account: Some("Work Account".into()),
                    account_id: Some("account-work".into()),
                },
            ])
        }

        fn create_folder(&self, folder: NewResource) -> Result<NoteFolder, BackendError> {
            if let Some(owner) = folder.owner.as_deref().filter(|owner| *owner != "iCloud") {
                return Err(BackendError::NotFound {
                    kind: "Notes account",
                    name: owner.into(),
                });
            }
            Ok(NoteFolder {
                id: "folder-created".into(),
                name: folder.name,
                account: Some("iCloud".into()),
                account_id: Some("account-icloud".into()),
            })
        }

        fn rename_folder(
            &self,
            folder: ResourceSelector,
            name: String,
        ) -> Result<NoteFolder, BackendError> {
            Ok(NoteFolder {
                id: selector_id(&folder, "folder-renamed"),
                name,
                account: Some("iCloud".into()),
                account_id: Some("account-icloud".into()),
            })
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
                due_time: reminder.due_time,
                due_time_zone: reminder.due_time_zone,
                due_at: None,
                priority: reminder.priority,
            })
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            Ok(vec![
                Reminder {
                    id: "reminder-1".into(),
                    title: "Review pull request".into(),
                    due: Some(date(2026, 10, 27)),
                    due_time: None,
                    due_time_zone: None,
                    due_at: None,
                    priority: Some(Priority::High),
                },
                Reminder {
                    id: "reminder-2".into(),
                    title: "Buy groceries".into(),
                    due: None,
                    due_time: None,
                    due_time_zone: None,
                    due_at: None,
                    priority: None,
                },
            ])
        }

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            let reminders = Reminders::list(self)?
                .into_iter()
                .enumerate()
                .map(|(index, reminder)| StoredReminder {
                    id: reminder.id,
                    list_id: if index == 0 {
                        "list-work".into()
                    } else {
                        "list-reminders".into()
                    },
                    title: reminder.title,
                    notes: None,
                    due: reminder.due,
                    due_components: None,
                    priority: reminder.priority,
                    completed: false,
                    modified: None,
                    alarms_present: false,
                    absolute_alarms: Some(Vec::new()),
                    recurrence_present: false,
                    url: None,
                    backup_partial: false,
                })
                .collect();
            Ok(ReminderSnapshot {
                lists: vec![
                    ReminderList {
                        id: "list-reminders".into(),
                        name: "Reminders".into(),
                        writable: true,
                    },
                    ReminderList {
                        id: "list-work".into(),
                        name: "Work".into(),
                        writable: false,
                    },
                ],
                reminders,
            })
        }

        fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
            self.snapshot(None)
        }

        fn lists(&self) -> Result<Vec<ReminderListInfo>, BackendError> {
            Ok(vec![
                ReminderListInfo {
                    id: "list-reminders".into(),
                    name: "Reminders".into(),
                    source: Some("iCloud".into()),
                    source_id: Some("source-icloud".into()),
                    kind: CalendarKind::Local,
                    writable: true,
                    default: true,
                },
                ReminderListInfo {
                    id: "list-work".into(),
                    name: "Work".into(),
                    source: Some("Work Account".into()),
                    source_id: Some("source-work".into()),
                    kind: CalendarKind::CalDav,
                    writable: false,
                    default: false,
                },
            ])
        }

        fn create_list(&self, list: NewResource) -> Result<ReminderListInfo, BackendError> {
            if let Some(owner) = list.owner.as_deref().filter(|owner| *owner != "iCloud") {
                return Err(BackendError::NotFound {
                    kind: "writable EventKit source",
                    name: owner.into(),
                });
            }
            Ok(ReminderListInfo {
                id: "list-created".into(),
                name: list.name,
                source: Some("iCloud".into()),
                source_id: Some("source-icloud".into()),
                kind: CalendarKind::CalDav,
                writable: true,
                default: false,
            })
        }

        fn rename_list(
            &self,
            list: ResourceSelector,
            name: String,
        ) -> Result<ReminderListInfo, BackendError> {
            Ok(ReminderListInfo {
                id: selector_id(&list, "list-renamed"),
                name,
                source: Some("iCloud".into()),
                source_id: Some("source-icloud".into()),
                kind: CalendarKind::CalDav,
                writable: true,
                default: false,
            })
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
                .map(|(id, title, start, all_day, calendar)| Event {
                    id: id.into(),
                    title: title.into(),
                    start,
                    all_day,
                    calendar: calendar.into(),
                })
                .collect())
        }

        fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
            Ok(vec![
                CalendarInfo {
                    id: "calendar-personal".into(),
                    name: "Personal".into(),
                    source: Some("iCloud".into()),
                    source_id: Some("source-icloud".into()),
                    kind: CalendarKind::Local,
                    writable: true,
                    default: true,
                },
                CalendarInfo {
                    id: "calendar-work".into(),
                    name: "Work".into(),
                    source: Some("Work Account".into()),
                    source_id: Some("source-work".into()),
                    kind: CalendarKind::CalDav,
                    writable: false,
                    default: false,
                },
            ])
        }

        fn create_calendar(&self, calendar: NewResource) -> Result<CalendarInfo, BackendError> {
            if let Some(owner) = calendar.owner.as_deref().filter(|owner| *owner != "iCloud") {
                return Err(BackendError::NotFound {
                    kind: "writable EventKit source",
                    name: owner.into(),
                });
            }
            Ok(CalendarInfo {
                id: "calendar-created".into(),
                name: calendar.name,
                source: Some("iCloud".into()),
                source_id: Some("source-icloud".into()),
                kind: CalendarKind::CalDav,
                writable: true,
                default: false,
            })
        }

        fn rename_calendar(
            &self,
            calendar: ResourceSelector,
            name: String,
        ) -> Result<CalendarInfo, BackendError> {
            Ok(CalendarInfo {
                id: selector_id(&calendar, "calendar-renamed"),
                name,
                source: Some("iCloud".into()),
                source_id: Some("source-icloud".into()),
                kind: CalendarKind::CalDav,
                writable: true,
                default: false,
            })
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

        fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
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

        fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
            unreachable!()
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

    fn snapshot_reminder(id: &str, title: &str, completed: bool) -> StoredReminder {
        StoredReminder {
            id: id.into(),
            list_id: "list-reminders".into(),
            title: title.into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            completed,
            modified: None,
            alarms_present: false,
            absolute_alarms: Some(Vec::new()),
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    fn reminder_snapshot(reminder: StoredReminder) -> ReminderSnapshot {
        ReminderSnapshot {
            lists: vec![ReminderList {
                id: "list-reminders".into(),
                name: "Reminders".into(),
                writable: true,
            }],
            reminders: vec![reminder],
        }
    }

    fn nib(args: &[&str]) -> String {
        let backends = fake_backends(&Built::default());
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now(), &Config::default())
            .unwrap()
            .unwrap();
        run(
            action,
            json,
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
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now(), &Config::default())
            .unwrap()
            .unwrap();
        run(
            action,
            json,
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
            due_time: None,
            due_time_zone: None,
            due_at: None,
            priority: None,
        }
    }

    fn event(id: &str, title: &str, day: Date, hour: i8) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start: at(day, hour, 0),
            all_day: false,
            calendar: "Work".into(),
        }
    }

    fn built_by(args: &[&str]) -> Vec<&'static str> {
        let built = Built::default();
        let backends = fake_backends(&built);
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now(), &Config::default())
            .unwrap()
            .unwrap();
        run(
            action,
            json,
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
            "partial": false,
            "unrecorded": true
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
        journal_run(args).0
    }

    fn journal_run(args: &[&str]) -> (String, Scripted) {
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
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now(), &Config::default())
            .unwrap()
            .unwrap();
        let output = run(action, json, &now(), &backends, Style::default(), &mut host).unwrap();
        (output, host)
    }

    #[test]
    fn log_lists_newest_entries_with_counts() {
        assert_snapshot!(journal_output(&["log"]), @r"
        ENTRY          WRITTEN               ACTION     STATUS   UNDO OF        UNDONE  ADDED  UPDATED  COMPLETED  DELETED  UNRECORDED
        newer-journal  2026-10-26T14:00:00Z  undo       applied  older-journal          0      0        0          1        0
        older-journal  2026-10-25T14:00:00Z  task_edit  applied                 yes     1      0        0          0        1
        ");
    }

    #[test]
    fn log_takes_the_journal_lock() {
        let (_, host) = journal_run(&["log"]);
        assert_eq!(host.journal_lock_count, 1);
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
            "deleted": 1,
            "unrecorded": 0
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
            "deleted": 0,
            "unrecorded": 1
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
    fn cancelled_undo_resolution_json_has_stable_fields() {
        assert_snapshot!(output::undo_resolution(
            "20261026T090100-journal-efgh",
            None,
            true,
            Style::default(),
        )
        .unwrap(), @r#"
        {
          "id": "20261026T090100-journal-efgh",
          "resolved": false,
          "applied": false
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
    fn today_shows_the_calendar_column_on_request() {
        let reminders = vec![reminder(
            "reminder-today",
            "Review pull request",
            Some(date(2026, 10, 26)),
        )];
        let mut lunch = event("event-lunch", "Lunch with Sam", date(2026, 10, 26), 12);
        lunch.calendar = "Personal".into();
        let events = vec![
            event("event-today", "Design review", date(2026, 10, 26), 14),
            lunch,
        ];
        assert_snapshot!(agenda(&["today", "--show-calendar"], reminders, events), @r"
        Due today
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-26

        Events
        START             TITLE           CALENDAR
        2026-10-26 14:00  Design review   Work
        2026-10-26 12:00  Lunch with Sam  Personal
        ");
    }

    #[test]
    fn today_splits_timed_reminders_at_the_current_time() {
        let mut past = reminder("past", "Morning task", Some(now().date()));
        past.due_time = Some(jiff::civil::time(9, 0, 0, 0));
        let mut future = reminder("future", "Afternoon task", Some(now().date()));
        future.due_time = Some(jiff::civil::time(15, 0, 0, 0));
        assert_snapshot!(agenda(&["today"], vec![past, future], Vec::new()), @r"
        Overdue
        TITLE         DUE               PRIORITY
        Morning task  2026-10-26 09:00

        Due today
        TITLE           DUE               PRIORITY
        Afternoon task  2026-10-26 15:00
        ");
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
              "due_time": null,
              "due_time_zone": null,
              "priority": null
            }
          ],
          "due_today": [
            {
              "id": "reminder-today",
              "title": "Review pull request",
              "due": "2026-10-26",
              "due_time": null,
              "due_time_zone": null,
              "priority": null
            }
          ],
          "events": [
            {
              "id": "event-today",
              "title": "Design review",
              "start": "2026-10-26T14:00:00-04:00",
              "all_day": false,
              "calendar": "Work"
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
    fn note_folders() {
        assert_snapshot!(nib(&["note", "folders"]), @r"
        NAME      ACCOUNT
        Personal  iCloud
        Work      Work Account
        ");
        assert_snapshot!(nib(&["--json", "note", "folders"]), @r#"
        [
          {
            "id": "folder-personal",
            "name": "Personal",
            "account": "iCloud",
            "account_id": "account-icloud"
          },
          {
            "id": "folder-work",
            "name": "Work",
            "account": "Work Account",
            "account_id": "account-work"
          }
        ]
        "#);
    }

    #[test]
    fn note_folder_mutations() {
        assert_snapshot!(
            nib(&["note", "folders", "create", "Archive", "--account", "iCloud"]),
            @r#"Created folder "Archive" in iCloud"#
        );
        assert_snapshot!(
            nib(&["--json", "note", "folders", "rename", "folder-work", "Projects", "--id"]),
            @r#"
        {
          "id": "folder-work",
          "name": "Projects",
          "account": "iCloud",
          "account_id": "account-icloud"
        }
        "#
        );
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
        assert_snapshot!(
            nib(&["t", "Call", "London", "tomorrow", "09:00"]),
            @r#"Added "Call London" (due 2026-10-27 09:00)"#
        );
        assert_snapshot!(
            nib(&["task", "add", "Call London", "--due", "9:30pm"]),
            @r#"Added "Call London" (due 2026-10-26 21:30)"#
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
          "due_time": null,
          "due_time_zone": null,
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
            "due_time": null,
            "due_time_zone": null,
            "priority": "high"
          },
          {
            "id": "reminder-2",
            "title": "Buy groceries",
            "due": null,
            "due_time": null,
            "due_time_zone": null,
            "priority": null
          }
        ]
        "#);
    }

    #[test]
    fn filtered_task_list_table() {
        assert_snapshot!(nib(&["task", "list", "--filter", "list:work"]), @r"
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-27  high
        ");
    }

    #[test]
    fn filtered_task_list_json() {
        assert_snapshot!(nib(&["task", "--filter", "pri:none", "--json"]), @r#"
        [
          {
            "id": "reminder-2",
            "title": "Buy groceries",
            "due": null,
            "due_time": null,
            "due_time_zone": null,
            "priority": null
          }
        ]
        "#);
    }

    #[test]
    fn done_filter_selects_the_completed_snapshot_only_when_positive() {
        struct DifferentSnapshots;

        impl Reminders for DifferentSnapshots {
            fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
                unreachable!()
            }

            fn list(&self) -> Result<Vec<Reminder>, BackendError> {
                unreachable!()
            }

            fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
                Ok(reminder_snapshot(snapshot_reminder(
                    "open",
                    "Open reminder",
                    false,
                )))
            }

            fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
                Ok(reminder_snapshot(snapshot_reminder(
                    "done",
                    "Completed reminder",
                    true,
                )))
            }

            fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
                unreachable!()
            }
        }

        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            || Box::new(DifferentSnapshots),
            || -> Box<dyn Calendar> { unreachable!() },
        );
        let render = |expression| {
            filtered_tasks(expression, false, &now(), &backends, Style::default()).unwrap()
        };

        assert_snapshot!(render("is:done"), @r"
        TITLE               DUE  PRIORITY
        Completed reminder
        ");
        assert_snapshot!(render("open"), @r"
        TITLE          DUE  PRIORITY
        Open reminder
        ");
        assert_snapshot!(render("!is:done"), @r"
        TITLE          DUE  PRIORITY
        Open reminder
        ");
    }

    #[test]
    fn filtered_snapshot_reconstructs_a_foreign_due_instant() {
        let local_zone = TimeZone::fixed(offset(-5));
        let mut stored = snapshot_reminder("tokyo", "Tokyo deadline", false);
        stored.due = Some(date(2026, 10, 27));
        stored.due_components = Some(ReminderDueComponents {
            year: Some(2026),
            month: Some(10),
            day: Some(27),
            hour: Some(9),
            minute: Some(0),
            time_zone: Some("Asia/Tokyo".into()),
            ..ReminderDueComponents::default()
        });

        let reminder = reminder_from_snapshot(stored, &local_zone);
        let expected = date(2026, 10, 27)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::get("Asia/Tokyo").unwrap())
            .unwrap()
            .timestamp();
        let now = date(2026, 10, 26)
            .at(20, 0, 0, 0)
            .to_zoned(local_zone)
            .unwrap();

        assert_eq!(reminder.due_at, Some(expected));
        assert!(output::reminder_is_overdue(&reminder, &now));
    }

    #[test]
    fn reminder_lists() {
        assert_snapshot!(nib(&["task", "lists"]), @r"
        NAME       SOURCE        TYPE    WRITABLE  DEFAULT
        Reminders  iCloud        local   yes       yes
        Work       Work Account  caldav
        ");
        assert_snapshot!(nib(&["--json", "task", "lists"]), @r#"
        [
          {
            "id": "list-reminders",
            "name": "Reminders",
            "source": "iCloud",
            "source_id": "source-icloud",
            "type": "local",
            "writable": true,
            "default": true
          },
          {
            "id": "list-work",
            "name": "Work",
            "source": "Work Account",
            "source_id": "source-work",
            "type": "caldav",
            "writable": false,
            "default": false
          }
        ]
        "#);
    }

    #[test]
    fn reminder_list_mutations() {
        assert_snapshot!(
            nib(&["task", "lists", "create", "Errands", "--source", "iCloud"]),
            @r#"Created list "Errands" in iCloud"#
        );
        assert_snapshot!(
            nib(&["--json", "task", "lists", "rename", "list-work", "Projects", "--id"]),
            @r#"
        {
          "id": "list-work",
          "name": "Projects",
          "source": "iCloud",
          "source_id": "source-icloud",
          "type": "caldav",
          "writable": true,
          "default": false
        }
        "#
        );
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
    fn cal_shows_the_calendar_column_on_request() {
        let mut dentist = event("event-dentist", "Dentist", date(2026, 10, 27), 8);
        dentist.calendar = "Personal".into();
        let events = vec![
            event("event-2", "Team sync", date(2026, 10, 26), 10),
            dentist,
            event("event-3", "Design review", date(2026, 10, 29), 14),
        ];
        assert_snapshot!(agenda(&["c", "list", "--week", "--show-calendar"], Vec::new(), events.clone()), @r"
        START             TITLE          CALENDAR
        2026-10-26 10:00  Team sync      Work
        2026-10-27 08:00  Dentist        Personal
        2026-10-29 14:00  Design review  Work
        ");
        assert_eq!(
            agenda(
                &["--json", "cal", "--week", "--show-calendar"],
                Vec::new(),
                events.clone()
            ),
            agenda(&["--json", "cal", "--week"], Vec::new(), events)
        );
    }

    #[test]
    fn configured_calendar_column_applies_to_cal_and_today() {
        let config = Config {
            cal: crate::domain::CalendarConfig {
                calendars: Vec::new(),
                show_calendar: true,
            },
            ..Config::default()
        };
        let run_with = |args: &[&str]| {
            let built = Built::default();
            let backends = fake_backends(&built);
            let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
            let json = cli.json;
            let action = cli.command.into_action(&now(), &config).unwrap().unwrap();
            run(
                action,
                json,
                &now(),
                &backends,
                Style::default(),
                &mut Scripted::new(),
            )
            .unwrap()
        };
        assert_snapshot!(run_with(&["cal"]), @r"
        START             TITLE      CALENDAR
        2026-10-26 10:00  Team sync  Work
        ");
        assert!(run_with(&["today"]).contains("CALENDAR"));
    }

    #[test]
    fn event_list_json() {
        assert_snapshot!(nib(&["cal", "--week", "--json"]), @r#"
        [
          {
            "id": "event-2",
            "title": "Team sync",
            "start": "2026-10-26T10:00:00-04:00",
            "all_day": false,
            "calendar": "Work"
          },
          {
            "id": "event-5",
            "title": "Offsite",
            "start": "2026-10-28T00:00:00-04:00",
            "all_day": true,
            "calendar": "Work"
          },
          {
            "id": "event-3",
            "title": "Design review",
            "start": "2026-10-29T14:30:00-04:00",
            "all_day": false,
            "calendar": "Work"
          }
        ]
        "#);
    }

    #[test]
    fn calendars() {
        assert_snapshot!(nib(&["cal", "calendars"]), @r"
        NAME      SOURCE        TYPE    WRITABLE  DEFAULT
        Personal  iCloud        local   yes       yes
        Work      Work Account  caldav
        ");
        assert_snapshot!(nib(&["--json", "cal", "calendars"]), @r#"
        [
          {
            "id": "calendar-personal",
            "name": "Personal",
            "source": "iCloud",
            "source_id": "source-icloud",
            "type": "local",
            "writable": true,
            "default": true
          },
          {
            "id": "calendar-work",
            "name": "Work",
            "source": "Work Account",
            "source_id": "source-work",
            "type": "caldav",
            "writable": false,
            "default": false
          }
        ]
        "#);
    }

    #[test]
    fn calendar_mutations() {
        assert_snapshot!(
            nib(&["cal", "calendars", "create", "Travel", "--source", "iCloud"]),
            @r#"Created calendar "Travel" in iCloud"#
        );
        assert_snapshot!(
            nib(&["--json", "cal", "calendars", "rename", "calendar-work", "Projects", "--id"]),
            @r#"
        {
          "id": "calendar-work",
          "name": "Projects",
          "source": "iCloud",
          "source_id": "source-icloud",
          "type": "caldav",
          "writable": true,
          "default": false
        }
        "#
        );
    }

    #[test]
    fn unsupported_resource_owners_are_errors() {
        assert!(matches!(
            Notes::create_folder(
                &Fake,
                NewResource {
                    name: "Archive".into(),
                    owner: Some("Missing".into()),
                }
            ),
            Err(BackendError::NotFound {
                kind: "Notes account",
                ..
            })
        ));
        assert!(matches!(
            Reminders::create_list(
                &Fake,
                NewResource {
                    name: "Errands".into(),
                    owner: Some("Missing".into()),
                }
            ),
            Err(BackendError::NotFound {
                kind: "writable EventKit source",
                ..
            })
        ));
        assert!(matches!(
            Calendar::create_calendar(
                &Fake,
                NewResource {
                    name: "Travel".into(),
                    owner: Some("Missing".into()),
                }
            ),
            Err(BackendError::NotFound {
                kind: "writable EventKit source",
                ..
            })
        ));
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
                    due_time: reminder.due_time,
                    due_time_zone: reminder.due_time_zone,
                    due_at: None,
                    priority: reminder.priority,
                })
            }

            fn list(&self) -> Result<Vec<Reminder>, BackendError> {
                unreachable!()
            }

            fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
                unreachable!()
            }

            fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
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
        let json = cli.json;
        let action = cli.command.into_action(&now(), &config).unwrap().unwrap();
        run(
            action,
            json,
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
                due_time: None,
                due_time_zone: None,
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

            fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
                unreachable!()
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
                show_calendar: false,
            },
            ..Config::default()
        };
        let cli = Cli::try_parse_from(["nib", "cal"]).unwrap();
        let json = cli.json;
        let action = cli.command.into_action(&now(), &config).unwrap().unwrap();
        run(
            action,
            json,
            &now(),
            &backends,
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap();

        assert_eq!(recorded.take().unwrap().calendars, ["Work", "Birthdays"]);
    }

    fn config_entries() -> Vec<ConfigEntry> {
        vec![
            ConfigEntry {
                key: ConfigKey::TaskList,
                value: ConfigValue::String("Work".into()),
                origin: ConfigOrigin::File("/Users/me/project/nib.toml".into()),
            },
            ConfigEntry {
                key: ConfigKey::TaskPriority,
                value: ConfigValue::Unset,
                origin: ConfigOrigin::Default,
            },
            ConfigEntry {
                key: ConfigKey::TaskParseInline,
                value: ConfigValue::Bool(true),
                origin: ConfigOrigin::Environment("NIB_TASK_PARSE_INLINE"),
            },
            ConfigEntry {
                key: ConfigKey::CalCalendars,
                value: ConfigValue::Strings(vec!["Work".into(), "Personal".into()]),
                origin: ConfigOrigin::Default,
            },
            ConfigEntry {
                key: ConfigKey::CalShowCalendar,
                value: ConfigValue::Bool(true),
                origin: ConfigOrigin::File("/Users/me/project/nib.toml".into()),
            },
            ConfigEntry {
                key: ConfigKey::NoteFolder,
                value: ConfigValue::String("Notes".into()),
                origin: ConfigOrigin::File("/Users/me/.config/nib/config.toml".into()),
            },
        ]
    }

    #[test]
    fn config_list_text_outputs_are_stable() {
        let entries = config_entries();
        assert_snapshot!(output::config_list(&entries, false, false, Style::default()).unwrap(), @r#"
        KEY                VALUE
        task.list          Work
        task.priority      <unset>
        task.parse_inline  true
        cal.calendars      ["Work", "Personal"]
        cal.show_calendar  true
        note.folder        Notes
        "#);
        assert_snapshot!(output::config_list(&entries, true, false, Style::default()).unwrap(), @r#"
        KEY                VALUE                 ORIGIN
        task.list          Work                  /Users/me/project/nib.toml
        task.priority      <unset>               default
        task.parse_inline  true                  NIB_TASK_PARSE_INLINE
        cal.calendars      ["Work", "Personal"]  default
        cal.show_calendar  true                  /Users/me/project/nib.toml
        note.folder        Notes                 /Users/me/.config/nib/config.toml
        "#);
    }

    #[test]
    fn config_list_json_outputs_are_stable() {
        let entries = config_entries();
        assert_snapshot!(output::config_list(&entries, true, true, Style::default()).unwrap(), @r#"
        [
          {
            "id": "task.list",
            "value": "Work",
            "origin": {
              "kind": "file",
              "path": "/Users/me/project/nib.toml"
            }
          },
          {
            "id": "task.priority",
            "value": null,
            "origin": {
              "kind": "default"
            }
          },
          {
            "id": "task.parse_inline",
            "value": true,
            "origin": {
              "kind": "env",
              "variable": "NIB_TASK_PARSE_INLINE"
            }
          },
          {
            "id": "cal.calendars",
            "value": [
              "Work",
              "Personal"
            ],
            "origin": {
              "kind": "default"
            }
          },
          {
            "id": "cal.show_calendar",
            "value": true,
            "origin": {
              "kind": "file",
              "path": "/Users/me/project/nib.toml"
            }
          },
          {
            "id": "note.folder",
            "value": "Notes",
            "origin": {
              "kind": "file",
              "path": "/Users/me/.config/nib/config.toml"
            }
          }
        ]
        "#);
    }

    #[test]
    fn config_list_json_ignores_the_origin_flag() {
        let entries = config_entries();
        assert_snapshot!(output::config_list(&entries, false, true, Style::default()).unwrap(), @r#"
        [
          {
            "id": "task.list",
            "value": "Work",
            "origin": {
              "kind": "file",
              "path": "/Users/me/project/nib.toml"
            }
          },
          {
            "id": "task.priority",
            "value": null,
            "origin": {
              "kind": "default"
            }
          },
          {
            "id": "task.parse_inline",
            "value": true,
            "origin": {
              "kind": "env",
              "variable": "NIB_TASK_PARSE_INLINE"
            }
          },
          {
            "id": "cal.calendars",
            "value": [
              "Work",
              "Personal"
            ],
            "origin": {
              "kind": "default"
            }
          },
          {
            "id": "cal.show_calendar",
            "value": true,
            "origin": {
              "kind": "file",
              "path": "/Users/me/project/nib.toml"
            }
          },
          {
            "id": "note.folder",
            "value": "Notes",
            "origin": {
              "kind": "file",
              "path": "/Users/me/.config/nib/config.toml"
            }
          }
        ]
        "#);
    }

    #[test]
    fn config_single_and_write_outputs_are_stable() {
        let entry = &config_entries()[0];
        let path = std::path::Path::new("/Users/me/.config/nib/config.toml");
        assert_snapshot!(output::config_get(entry, false, Style::default()).unwrap(), @"Work");
        assert_snapshot!(output::config_get(entry, true, Style::default()).unwrap(), @r#"
        {
          "id": "task.list",
          "value": "Work"
        }
        "#);
        assert_snapshot!(output::config_set("task.list", &entry.value, path, false, Style::default()).unwrap(), @"Set task.list in /Users/me/.config/nib/config.toml");
        assert_snapshot!(output::config_set("task.list", &entry.value, path, true, Style::default()).unwrap(), @r#"
        {
          "id": "task.list",
          "value": "Work",
          "path": "/Users/me/.config/nib/config.toml"
        }
        "#);
        assert_snapshot!(output::config_unset("task.list", path, true, false, Style::default()).unwrap(), @"Unset task.list in /Users/me/.config/nib/config.toml");
        assert_snapshot!(output::config_unset("task.list", path, true, true, Style::default()).unwrap(), @r#"
        {
          "id": "task.list",
          "path": "/Users/me/.config/nib/config.toml",
          "changed": true
        }
        "#);
        assert_snapshot!(output::config_unset("task.list", path, false, false, Style::default()).unwrap(), @"task.list isn't set in /Users/me/.config/nib/config.toml; nothing changed");
        assert_snapshot!(output::config_unset("task.list", path, false, true, Style::default()).unwrap(), @r#"
        {
          "id": "task.list",
          "path": "/Users/me/.config/nib/config.toml",
          "changed": false
        }
        "#);
        assert_snapshot!(output::config_path(path, false, Style::default()).unwrap(), @"/Users/me/.config/nib/config.toml");
        assert_snapshot!(output::config_path(path, true, Style::default()).unwrap(), @r#"
        {
          "id": "user",
          "path": "/Users/me/.config/nib/config.toml"
        }
        "#);
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
            &[],
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
        ✓ no configuration lock files are left behind
        ! $VISUAL uses `code` without a wait flag
          Fix: Set VISUAL="code --wait".
        "###);
    }

    #[test]
    fn doctor_warns_about_lock_files_from_older_versions() {
        let checks = crate::app::doctor::checks(
            &FakeDoctor,
            &[],
            None,
            &[crate::app::doctor::ConfigLockFile {
                path: "/Users/me/project/nib.toml.lock".into(),
            }],
        );
        let lock = checks
            .iter()
            .find(|check| check.id == "config_lock")
            .unwrap();
        assert_eq!(lock.status, crate::app::doctor::CheckStatus::Warning);
        assert_eq!(
            lock.message,
            "leftover lock file from an older nib: /Users/me/project/nib.toml.lock"
        );
        assert_eq!(
            lock.fix.as_deref(),
            Some("Remove /Users/me/project/nib.toml.lock; nib no longer uses it.")
        );
    }

    fn legacy_config_lock_checks() -> Vec<crate::app::doctor::Check> {
        crate::app::doctor::checks(
            &FakeDoctor,
            &[],
            None,
            &[
                crate::app::doctor::ConfigLockFile {
                    path: "/Users/me/project/nib.toml.lock".into(),
                },
                crate::app::doctor::ConfigLockFile {
                    path: "/Users/me/project/.nib.local.toml.lock".into(),
                },
            ],
        )
    }

    #[test]
    fn doctor_text_reports_leftover_config_locks() {
        let checks = legacy_config_lock_checks();
        assert_snapshot!(output::doctor(&checks, false, Style::default()).unwrap(), @r###"
        ✓ macOS 26.1 is supported
        ✓ macOS attributes permissions to Ghostty
        ✓ Ghostty has full access to Reminders
        ✗ Ghostty access to Calendars is denied
          Fix: Turn on Ghostty in System Settings > Privacy & Security > Calendars.
        ! Apple Notes Automation access is checked on first use
          Fix: When prompted, allow Ghostty to control Notes; later changes are in System Settings > Privacy & Security > Automation.
        ✓ no config files loaded; built-in defaults are valid
        ! leftover lock files from an older nib: /Users/me/project/nib.toml.lock, /Users/me/project/.nib.local.toml.lock
          Fix: Remove /Users/me/project/nib.toml.lock, /Users/me/project/.nib.local.toml.lock; nib no longer uses them.
        ! $VISUAL uses `code` without a wait flag
          Fix: Set VISUAL="code --wait".
        "###);
    }

    #[test]
    fn doctor_json_reports_leftover_config_locks() {
        let checks = legacy_config_lock_checks();
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
              "id": "config",
              "status": "pass",
              "message": "no config files loaded; built-in defaults are valid",
              "fix": null
            },
            {
              "id": "config_lock",
              "status": "warning",
              "message": "leftover lock files from an older nib: /Users/me/project/nib.toml.lock, /Users/me/project/.nib.local.toml.lock",
              "fix": "Remove /Users/me/project/nib.toml.lock, /Users/me/project/.nib.local.toml.lock; nib no longer uses them."
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

    #[test]
    fn doctor_json_has_stable_check_objects() {
        let checks = crate::app::doctor::checks(
            &FakeDoctor,
            &[crate::app::doctor::ConfigFileCheck {
                path: "/Users/me/project/nib.toml".into(),
                error: None,
            }],
            None,
            &[],
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
              "id": "config_lock",
              "status": "pass",
              "message": "no configuration lock files are left behind",
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

#[cfg(test)]
mod quick_tests {
    use clap::Parser;
    use insta::assert_snapshot;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::infra::host::Scripted;
    use crate::{Cli, Config};

    fn nib(args: &[&str]) -> String {
        let now = date(2026, 10, 26)
            .at(9, 30, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now, &Config::default())
            .unwrap()
            .unwrap();
        run(
            action,
            json,
            &now,
            &crate::infra::demo::backends(&now),
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap()
    }

    #[test]
    fn done_prints_each_completed_reminder() {
        assert_snapshot!(nib(&["task", "done", "rent", "reminder-4"]), @r#"
        Completed "Pay rent"
        Completed "Buy groceries"
        "#);
        assert_snapshot!(nib(&["task", "done", "rent", "--json"]), @r#"
        [
          {
            "id": "reminder-2",
            "title": "Pay rent",
            "list": "Reminders",
            "completed": true,
            "priority": "medium",
            "change": "completed"
          }
        ]
        "#);
    }

    #[test]
    fn reopen_prints_each_reopened_reminder() {
        assert_snapshot!(nib(&["task", "reopen", "dentist"]), @r#"Reopened "Call the dentist""#);
        assert_snapshot!(nib(&["task", "reopen", "reminder-6", "--json"]), @r#"
        [
          {
            "id": "reminder-6",
            "title": "Call the dentist",
            "list": "Reminders",
            "completed": false,
            "priority": null,
            "change": "reopened"
          }
        ]
        "#);
    }

    #[test]
    fn demo_completed_reminders_are_listed_by_is_done() {
        assert_snapshot!(nib(&["task", "list", "-f", "is:done"]), @r"
        TITLE                DUE  PRIORITY
        Call the dentist
        File expense report
        ");
    }

    #[test]
    fn priority_prints_the_new_priority_or_that_it_was_cleared() {
        assert_snapshot!(nib(&["task", "priority", "high", "groceries"]), @r#"Set "Buy groceries" to high priority"#);
        assert_snapshot!(nib(&["task", "priority", "none", "rent"]), @r#"Cleared priority on "Pay rent""#);
        assert_snapshot!(nib(&["task", "priority", "high", "review"]), @"No changes.");
        assert_snapshot!(nib(&["task", "priority", "low", "groceries", "--json"]), @r#"
        [
          {
            "id": "reminder-4",
            "title": "Buy groceries",
            "list": "Reminders",
            "completed": false,
            "priority": "low",
            "change": "priority"
          }
        ]
        "#);
        assert_snapshot!(nib(&["task", "priority", "high", "review", "--json"]), @"[]");
    }

    #[test]
    fn move_prints_the_destination_list() {
        assert_snapshot!(nib(&["task", "move", "Work", "rent", "passport"]), @r#"
        Moved "Pay rent" to Work
        Moved "Renew passport" to Work
        "#);
        assert_snapshot!(nib(&["task", "move", "Work", "rent", "--json"]), @r#"
        [
          {
            "id": "reminder-2",
            "title": "Pay rent",
            "list": "Work",
            "completed": false,
            "priority": "medium",
            "change": "moved"
          }
        ]
        "#);
    }
}

#[cfg(all(test, feature = "tui"))]
mod pick_tests {
    use insta::assert_snapshot;
    use jiff::civil::{date, time};

    use crate::cli::output::{self, PickedReminder, Style};
    use crate::domain::Priority;

    fn picked() -> Vec<PickedReminder<'static>> {
        vec![
            PickedReminder {
                id: "reminder-3",
                title: "Renew passport",
                list: Some("Reminders"),
                due: Some(date(2026, 11, 9)),
                due_time: Some(time(9, 0, 0, 0)),
                priority: Some(Priority::Low),
                completed: false,
            },
            PickedReminder {
                id: "reminder-5",
                title: "Water plants",
                list: None,
                due: None,
                due_time: None,
                priority: None,
                completed: false,
            },
        ]
    }

    #[test]
    fn pick_prints_one_id_per_line() {
        assert_snapshot!(output::picked(&picked(), false, Style::default()).unwrap(), @r"
        reminder-3
        reminder-5
        ");
    }

    #[test]
    fn pick_json_lists_each_chosen_reminder() {
        assert_snapshot!(output::picked(&picked(), true, Style::default()).unwrap(), @r#"
        [
          {
            "id": "reminder-3",
            "title": "Renew passport",
            "list": "Reminders",
            "due": "2026-11-09",
            "due_time": "09:00",
            "priority": "low",
            "completed": false
          },
          {
            "id": "reminder-5",
            "title": "Water plants",
            "list": null,
            "due": null,
            "due_time": null,
            "priority": null,
            "completed": false
          }
        ]
        "#);
    }
}

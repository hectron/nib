use jiff::civil::{Date, Time};
use jiff::{ToSpan, Zoned};

use crate::domain::{
    ApplyResult, BackendError, Backends, Calendar, CalendarInfo, CalendarKind, Event, EventQuery,
    FieldEdit, NewNote, NewReminder, NewResource, Note, NoteFolder, NoteQuery, Notes, Priority,
    Reminder, ReminderChange, ReminderList, ReminderListInfo, ReminderSnapshot, Reminders,
    ResourceSelector, StoredReminder,
};

const DEMO_LIST: &str = "demo-list";
const DEMO_WORK_LIST: &str = "demo-list-work";

#[derive(Clone, Copy)]
struct DemoResource {
    id: &'static str,
    name: &'static str,
    owner: &'static str,
    owner_id: &'static str,
}

const NOTE_FOLDERS: &[DemoResource] = &[
    DemoResource {
        id: "folder-personal",
        name: "Personal",
        owner: "iCloud",
        owner_id: "account-icloud",
    },
    DemoResource {
        id: "folder-work",
        name: "Work",
        owner: "iCloud",
        owner_id: "account-icloud",
    },
];
const REMINDER_LISTS: &[DemoResource] = &[
    DemoResource {
        id: DEMO_LIST,
        name: "Reminders",
        owner: "iCloud",
        owner_id: "source-icloud",
    },
    DemoResource {
        id: DEMO_WORK_LIST,
        name: "Work",
        owner: "iCloud",
        owner_id: "source-icloud",
    },
];
const CALENDARS: &[DemoResource] = &[
    DemoResource {
        id: "calendar-personal",
        name: "Personal",
        owner: "iCloud",
        owner_id: "source-icloud",
    },
    DemoResource {
        id: "calendar-work",
        name: "Work",
        owner: "Work Account",
        owner_id: "source-work",
    },
];

pub fn backends(now: &Zoned) -> Backends {
    let (notes, reminders, calendar) = (Demo(now.clone()), Demo(now.clone()), Demo(now.clone()));
    Backends::new(
        move || Box::new(notes),
        move || Box::new(reminders),
        move || Box::new(calendar),
    )
}

struct Demo(Zoned);

impl Demo {
    fn day(&self, offset: i32, app: &'static str) -> Result<Date, BackendError> {
        self.0
            .date()
            .checked_add(offset.days())
            .map_err(|error| failed(app, &error))
    }

    fn at(&self, offset: i32, hour: i8, minute: i8) -> Result<Zoned, BackendError> {
        self.day(offset, "Calendar")?
            .at(hour, minute, 0, 0)
            .to_zoned(self.0.time_zone().clone())
            .map_err(|error| failed("Calendar", &error))
    }
}

fn failed(app: &'static str, error: &jiff::Error) -> BackendError {
    BackendError::Failed {
        app,
        message: error.to_string(),
    }
}

impl Notes for Demo {
    fn create(&self, note: NewNote) -> Result<Note, BackendError> {
        Ok(Note {
            id: "note-5".into(),
            title: note.title,
            folder: note.folder,
            modified: Some(self.0.timestamp()),
            locked: false,
        })
    }

    fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError> {
        let notes = [
            ("note-1", "Groceries", "Personal"),
            ("note-2", "Q4 planning", "Work"),
            ("note-3", "Standup notes", "Work"),
            ("note-4", "Book ideas", "Personal"),
        ];
        Ok(notes
            .into_iter()
            .filter(|(_, _, folder)| query.folder.as_deref().is_none_or(|want| want == *folder))
            .map(|(id, title, folder)| Note {
                id: id.into(),
                title: title.into(),
                folder: Some(folder.into()),
                modified: Some(self.0.timestamp()),
                locked: false,
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
                account: Some("iCloud".into()),
                account_id: Some("account-icloud".into()),
            },
        ])
    }

    fn create_folder(&self, folder: NewResource) -> Result<NoteFolder, BackendError> {
        let account = demo_owner(
            folder.owner.as_deref(),
            "Notes account",
            &[("iCloud", "account-icloud")],
            "account-icloud",
        )?;
        ensure_demo_unique(NOTE_FOLDERS, account.1, &folder.name, None, "folder")?;
        Ok(NoteFolder {
            id: "folder-created".into(),
            name: folder.name,
            account: Some(account.0.into()),
            account_id: Some(account.1.into()),
        })
    }

    fn rename_folder(
        &self,
        folder: ResourceSelector,
        name: String,
    ) -> Result<NoteFolder, BackendError> {
        let selected = demo_resource(&folder, "folder", NOTE_FOLDERS)?;
        ensure_demo_unique(
            NOTE_FOLDERS,
            selected.owner_id,
            &name,
            Some(selected.id),
            "folder",
        )?;
        Ok(NoteFolder {
            id: selected.id.into(),
            name,
            account: Some(selected.owner.into()),
            account_id: Some(selected.owner_id.into()),
        })
    }
}

impl Reminders for Demo {
    fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
        if let Some(name) = reminder
            .list
            .as_deref()
            .filter(|name| !REMINDER_LISTS.iter().any(|list| list.name == *name))
        {
            return Err(BackendError::NotFound {
                kind: "writable reminder list",
                name: name.into(),
            });
        }
        let due_at = reminder
            .due
            .zip(reminder.due_time)
            .and_then(|(date, time)| {
                date.to_datetime(time)
                    .to_zoned(self.0.time_zone().clone())
                    .ok()
            })
            .map(|due| due.timestamp());
        Ok(Reminder {
            id: "reminder-5".into(),
            title: reminder.title,
            due: reminder.due,
            due_time: reminder.due_time,
            due_time_zone: reminder.due_time_zone,
            due_at,
            priority: reminder.priority,
        })
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        let reminder = |id: &str,
                        title: &str,
                        due: Option<i32>,
                        time: Option<(i8, i8)>,
                        zone: Option<&str>,
                        priority| {
            let due = due.map(|days| self.day(days, "Reminders")).transpose()?;
            let due_time = time
                .map(|(hour, minute)| Time::new(hour, minute, 0, 0).expect("demo times are valid"));
            let due_at = due
                .zip(due_time)
                .and_then(|(date, time)| {
                    date.to_datetime(time)
                        .to_zoned(self.0.time_zone().clone())
                        .ok()
                })
                .map(|due| due.timestamp());
            Ok(Reminder {
                id: id.into(),
                title: title.into(),
                due,
                due_time,
                due_time_zone: zone.map(ToOwned::to_owned),
                due_at,
                priority,
            })
        };
        [
            reminder(
                "reminder-1",
                "Review pull request",
                Some(0),
                None,
                None,
                Some(Priority::High),
            ),
            reminder(
                "reminder-2",
                "Pay rent",
                Some(-1),
                None,
                None,
                Some(Priority::Medium),
            ),
            reminder(
                "reminder-3",
                "Renew passport",
                Some(14),
                Some((9, 0)),
                self.0.time_zone().iana_name(),
                Some(Priority::Low),
            ),
            reminder("reminder-4", "Buy groceries", None, None, None, None),
        ]
        .into_iter()
        .collect()
    }

    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
        let lists: Vec<ReminderList> = REMINDER_LISTS
            .iter()
            .filter(|resource| list.is_none_or(|name| name == resource.name))
            .map(|resource| ReminderList {
                id: resource.id.into(),
                name: resource.name.into(),
                writable: true,
            })
            .collect();
        if let Some(name) = list
            && lists.is_empty()
        {
            return Err(BackendError::NotFound {
                kind: "reminder list",
                name: name.into(),
            });
        }
        let reminders = Reminders::list(self)?
            .into_iter()
            .map(|r| {
                let due_components = r.due.map(|date| crate::domain::ReminderDueComponents {
                    year: Some(date.year().into()),
                    month: Some(date.month().into()),
                    day: Some(date.day().into()),
                    hour: r.due_time.map(|time| i64::from(time.hour())),
                    minute: r.due_time.map(|time| i64::from(time.minute())),
                    time_zone: r.due_time_zone.clone(),
                    ..crate::domain::ReminderDueComponents::default()
                });
                StoredReminder {
                    id: r.id,
                    list_id: DEMO_LIST.into(),
                    title: r.title,
                    notes: None,
                    due: r.due,
                    due_components,
                    priority: r.priority,
                    completed: false,
                    modified: None,
                    alarms_present: false,
                    absolute_alarms: Some(Vec::new()),
                    recurrence_present: false,
                    url: None,
                    backup_partial: false,
                }
            })
            .filter(|reminder| lists.iter().any(|list| list.id == reminder.list_id))
            .collect();
        Ok(ReminderSnapshot { lists, reminders })
    }

    fn lists(&self) -> Result<Vec<ReminderListInfo>, BackendError> {
        Ok(REMINDER_LISTS
            .iter()
            .map(|resource| ReminderListInfo {
                id: resource.id.into(),
                name: resource.name.into(),
                source: Some(resource.owner.into()),
                source_id: Some(resource.owner_id.into()),
                kind: CalendarKind::Local,
                writable: true,
                default: resource.id == DEMO_LIST,
            })
            .collect())
    }

    fn reminder(&self, id: &str) -> Result<Option<StoredReminder>, BackendError> {
        Ok(self
            .snapshot(None)?
            .reminders
            .into_iter()
            .chain(self.completed_reminders()?)
            .find(|reminder| reminder.id == id))
    }

    fn completed_reminders(&self) -> Result<Vec<StoredReminder>, BackendError> {
        Ok(vec![StoredReminder {
            id: "reminder-6".into(),
            list_id: DEMO_LIST.into(),
            title: "Call the dentist".into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            completed: true,
            modified: None,
            alarms_present: false,
            absolute_alarms: Some(Vec::new()),
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }])
    }

    fn create_list(&self, list: NewResource) -> Result<ReminderListInfo, BackendError> {
        let source = demo_owner(
            list.owner.as_deref(),
            "EventKit source",
            &[("iCloud", "source-icloud")],
            "source-icloud",
        )?;
        ensure_demo_unique(REMINDER_LISTS, source.1, &list.name, None, "reminder list")?;
        Ok(ReminderListInfo {
            id: "list-created".into(),
            name: list.name,
            source: Some(source.0.into()),
            source_id: Some(source.1.into()),
            kind: CalendarKind::Local,
            writable: true,
            default: false,
        })
    }

    fn rename_list(
        &self,
        list: ResourceSelector,
        name: String,
    ) -> Result<ReminderListInfo, BackendError> {
        let selected = demo_resource(&list, "reminder list", REMINDER_LISTS)?;
        ensure_demo_unique(
            REMINDER_LISTS,
            selected.owner_id,
            &name,
            Some(selected.id),
            "reminder list",
        )?;
        Ok(ReminderListInfo {
            id: selected.id.into(),
            name,
            source: Some(selected.owner.into()),
            source_id: Some(selected.owner_id.into()),
            kind: CalendarKind::Local,
            writable: true,
            default: selected.id == DEMO_LIST,
        })
    }

    fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
        Ok(ApplyResult {
            written: changes
                .iter()
                .enumerate()
                .map(|(index, change)| demo_written(index, change))
                .collect(),
            unrecorded: Vec::new(),
        })
    }
}

fn demo_written(index: usize, change: &ReminderChange) -> Option<StoredReminder> {
    match change {
        ReminderChange::Add {
            list_id,
            title,
            notes,
            due,
            due_time,
            due_time_zone,
            priority,
        } => Some(StoredReminder {
            id: format!("demo-added-{index}"),
            list_id: list_id.clone(),
            title: title.clone(),
            notes: notes.clone(),
            due: *due,
            due_components: due.map(|date| crate::domain::ReminderDueComponents {
                year: Some(date.year().into()),
                month: Some(date.month().into()),
                day: Some(date.day().into()),
                hour: due_time.map(|time| i64::from(time.hour())),
                minute: due_time.map(|time| i64::from(time.minute())),
                time_zone: due_time_zone.clone(),
                ..crate::domain::ReminderDueComponents::default()
            }),
            priority: *priority,
            completed: false,
            modified: None,
            alarms_present: false,
            absolute_alarms: Some(Vec::new()),
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }),
        ReminderChange::Restore { before } => {
            let mut written = before.clone();
            written.id = format!("demo-restored-{index}");
            Some(written)
        }
        ReminderChange::Update { before, edit } => {
            let mut written = before.clone();
            if let Some(title) = &edit.title {
                written.title.clone_from(title);
            }
            demo_field(&mut written.notes, &edit.notes);
            demo_field(&mut written.due, &edit.due);
            if let Some(parts) = &edit.due_components {
                written.due_components = Some((**parts).clone());
            }
            demo_field(&mut written.priority, &edit.priority);
            if let Some(list_id) = &edit.list_id {
                written.list_id.clone_from(list_id);
            }
            if let Some(completed) = edit.completed {
                written.completed = completed;
            }
            Some(written)
        }
        ReminderChange::Delete { .. } => None,
    }
}

fn demo_field<T: Clone>(value: &mut Option<T>, edit: &FieldEdit<T>) {
    match edit {
        FieldEdit::Unchanged => {}
        FieldEdit::Set(new) => *value = Some(new.clone()),
        FieldEdit::Remove => *value = None,
    }
}

impl Calendar for Demo {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
        let events = [
            (
                "event-1",
                "Book club",
                self.at(-1, 18, 0)?,
                false,
                "Personal",
            ),
            ("event-2", "Standup", self.at(0, 9, 30)?, false, "Work"),
            (
                "event-3",
                "Design review",
                self.at(0, 14, 0)?,
                false,
                "Work",
            ),
            (
                "event-4",
                "Lunch with Sam",
                self.at(2, 12, 0)?,
                false,
                "Personal",
            ),
            ("event-5", "Team offsite", self.at(3, 0, 0)?, true, "Work"),
            (
                "event-6",
                "Sprint planning",
                self.at(5, 10, 0)?,
                false,
                "Work",
            ),
            ("event-7", "Retro", self.at(9, 16, 0)?, false, "Work"),
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
                    && (query.calendars.is_empty() || query.calendars.iter().any(|c| c == calendar))
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
                writable: true,
                default: false,
            },
        ])
    }

    fn create_calendar(&self, calendar: NewResource) -> Result<CalendarInfo, BackendError> {
        let source = demo_owner(
            calendar.owner.as_deref(),
            "EventKit source",
            &[("iCloud", "source-icloud"), ("Work Account", "source-work")],
            "source-icloud",
        )?;
        ensure_demo_unique(CALENDARS, source.1, &calendar.name, None, "calendar")?;
        Ok(CalendarInfo {
            id: "calendar-created".into(),
            name: calendar.name,
            source: Some(source.0.into()),
            source_id: Some(source.1.into()),
            kind: if source.1 == "source-work" {
                CalendarKind::CalDav
            } else {
                CalendarKind::Local
            },
            writable: true,
            default: false,
        })
    }

    fn rename_calendar(
        &self,
        calendar: ResourceSelector,
        name: String,
    ) -> Result<CalendarInfo, BackendError> {
        let selected = demo_resource(&calendar, "calendar", CALENDARS)?;
        ensure_demo_unique(
            CALENDARS,
            selected.owner_id,
            &name,
            Some(selected.id),
            "calendar",
        )?;
        Ok(CalendarInfo {
            id: selected.id.into(),
            name,
            source: Some(selected.owner.into()),
            source_id: Some(selected.owner_id.into()),
            kind: if selected.owner_id == "source-work" {
                CalendarKind::CalDav
            } else {
                CalendarKind::Local
            },
            writable: true,
            default: selected.id == "calendar-personal",
        })
    }
}

fn demo_owner<'a>(
    requested: Option<&str>,
    kind: &'static str,
    owners: &'a [(&'a str, &'a str)],
    default_id: &str,
) -> Result<(&'a str, &'a str), BackendError> {
    let selector = requested.unwrap_or(default_id);
    owners
        .iter()
        .copied()
        .find(|(name, id)| selector == *name || selector == *id)
        .ok_or_else(|| BackendError::NotFound {
            kind,
            name: selector.into(),
        })
}

fn demo_resource<'a>(
    selector: &ResourceSelector,
    kind: &'static str,
    resources: &'a [DemoResource],
) -> Result<&'a DemoResource, BackendError> {
    resources
        .iter()
        .find(|resource| match selector {
            ResourceSelector::Id(id) => resource.id == id,
            ResourceSelector::Name { name, owner } => {
                resource.name == name
                    && owner
                        .as_deref()
                        .is_none_or(|owner| owner == resource.owner || owner == resource.owner_id)
            }
        })
        .ok_or_else(|| BackendError::NotFound {
            kind,
            name: match selector {
                ResourceSelector::Id(id) => id,
                ResourceSelector::Name { name, .. } => name,
            }
            .clone(),
        })
}

fn ensure_demo_unique(
    resources: &[DemoResource],
    owner_id: &str,
    name: &str,
    current_id: Option<&str>,
    kind: &'static str,
) -> Result<(), BackendError> {
    if resources.iter().any(|resource| {
        resource.owner_id == owner_id && resource.name == name && current_id != Some(resource.id)
    }) {
        let owner = resources
            .iter()
            .find(|resource| resource.owner_id == owner_id)
            .map_or(owner_id, |resource| resource.owner);
        Err(BackendError::AlreadyExists {
            kind,
            name: name.into(),
            owner: format!("{owner} [{owner_id}]"),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use insta::assert_snapshot;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::app::commands::run;
    use crate::cli::Cli;
    use crate::cli::output::Style;
    use crate::infra::host::Scripted;

    fn nib(args: &[&str]) -> String {
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        let json = cli.json;
        let action = cli
            .command
            .into_action(&now, &crate::domain::Config::default())
            .unwrap()
            .unwrap();
        run(
            action,
            json,
            &now,
            &backends(&now),
            Style::default(),
            &mut Scripted::new(),
        )
        .unwrap()
    }

    #[test]
    fn calendar_follows_today() {
        assert_snapshot!(nib(&["cal"]), @r"
        START             TITLE
        2026-10-26 09:30  Standup
        2026-10-26 14:00  Design review
        ");
        assert_snapshot!(nib(&["cal", "--week"]), @r"
        START               TITLE
        2026-10-26 09:30    Standup
        2026-10-26 14:00    Design review
        2026-10-28 12:00    Lunch with Sam
        2026-10-29 all day  Team offsite
        2026-10-31 10:00    Sprint planning
        ");
    }

    #[test]
    fn reminders_are_due_relative_to_today() {
        assert_snapshot!(nib(&["t"]), @r"
        TITLE                DUE               PRIORITY
        Review pull request  2026-10-26        high
        Pay rent             2026-10-25        medium
        Renew passport       2026-11-09 09:00  low
        Buy groceries
        ");
    }

    #[test]
    fn today_is_a_useful_agenda() {
        assert_snapshot!(nib(&["today"]), @r"
        Overdue
        TITLE     DUE         PRIORITY
        Pay rent  2026-10-25  medium

        Due today
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-26  high

        Events
        START             TITLE
        2026-10-26 09:30  Standup
        2026-10-26 14:00  Design review
        ");
    }

    #[test]
    fn notes_filter_by_folder() {
        assert_snapshot!(nib(&["n", "-f", "Personal"]), @r"
        TITLE       FOLDER    LOCKED
        Groceries   Personal
        Book ideas  Personal
        ");
    }

    fn demo_backend() -> Demo {
        Demo(
            date(2026, 10, 26)
                .at(9, 0, 0, 0)
                .to_zoned(TimeZone::fixed(offset(-4)))
                .unwrap(),
        )
    }

    #[test]
    fn resource_mutations_reject_only_same_owner_collisions() {
        let demo = demo_backend();
        let folder = Notes::create_folder(
            &demo,
            NewResource {
                name: "Work".into(),
                owner: Some("account-icloud".into()),
            },
        );
        assert!(matches!(
            folder,
            Err(BackendError::AlreadyExists { kind: "folder", .. })
        ));

        let list = Reminders::create_list(
            &demo,
            NewResource {
                name: "Reminders".into(),
                owner: Some("iCloud".into()),
            },
        );
        assert!(matches!(
            list,
            Err(BackendError::AlreadyExists {
                kind: "reminder list",
                ..
            })
        ));

        let calendar = Calendar::create_calendar(
            &demo,
            NewResource {
                name: "Work".into(),
                owner: Some("source-icloud".into()),
            },
        )
        .unwrap();
        assert_eq!(calendar.source_id.as_deref(), Some("source-icloud"));
    }

    #[test]
    fn resource_renames_preserve_selected_metadata_and_allow_self_rename() {
        let demo = demo_backend();
        let folder = Notes::rename_folder(
            &demo,
            ResourceSelector::Name {
                name: "Work".into(),
                owner: Some("iCloud".into()),
            },
            "Work".into(),
        )
        .unwrap();
        assert_eq!(folder.id, "folder-work");

        let collision = Notes::rename_folder(
            &demo,
            ResourceSelector::Id("folder-personal".into()),
            "Work".into(),
        );
        assert!(matches!(
            collision,
            Err(BackendError::AlreadyExists { kind: "folder", .. })
        ));

        let calendar = Calendar::rename_calendar(
            &demo,
            ResourceSelector::Name {
                name: "Work".into(),
                owner: Some("source-work".into()),
            },
            "Projects".into(),
        )
        .unwrap();
        assert_eq!(calendar.id, "calendar-work");
        assert_eq!(calendar.source.as_deref(), Some("Work Account"));
        assert_eq!(calendar.source_id.as_deref(), Some("source-work"));
        assert!(matches!(calendar.kind, CalendarKind::CalDav));
        assert!(!calendar.default);
    }
}

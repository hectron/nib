use jiff::civil::Date;
use jiff::{ToSpan, Zoned};

use crate::domain::{
    ApplyResult, BackendError, Backends, Calendar, Event, EventQuery, FieldEdit, NewNote,
    NewReminder, Note, NoteQuery, Notes, Priority, Reminder, ReminderChange, ReminderList,
    ReminderSnapshot, Reminders, StoredReminder,
};

const DEMO_LIST: &str = "demo-list";

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
}

impl Reminders for Demo {
    fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
        if let Some(name) = reminder.list.as_deref().filter(|name| *name != "Reminders") {
            return Err(BackendError::NotFound {
                kind: "writable reminder list",
                name: name.into(),
            });
        }
        Ok(Reminder {
            id: "reminder-5".into(),
            title: reminder.title,
            due: reminder.due,
            priority: reminder.priority,
        })
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        let reminder = |id: &str, title: &str, due: Option<i32>, priority| {
            Ok(Reminder {
                id: id.into(),
                title: title.into(),
                due: due.map(|days| self.day(days, "Reminders")).transpose()?,
                priority,
            })
        };
        [
            reminder(
                "reminder-1",
                "Review pull request",
                Some(0),
                Some(Priority::High),
            ),
            reminder("reminder-2", "Pay rent", Some(-1), Some(Priority::Medium)),
            reminder(
                "reminder-3",
                "Renew passport",
                Some(14),
                Some(Priority::Low),
            ),
            reminder("reminder-4", "Buy groceries", None, None),
        ]
        .into_iter()
        .collect()
    }

    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
        if let Some(name) = list.filter(|name| *name != "Reminders") {
            return Err(BackendError::NotFound {
                kind: "reminder list",
                name: name.into(),
            });
        }
        let reminders = Reminders::list(self)?
            .into_iter()
            .map(|r| StoredReminder {
                id: r.id,
                list_id: DEMO_LIST.into(),
                title: r.title,
                notes: None,
                due: r.due,
                due_components: None,
                priority: r.priority,
                completed: false,
                modified: None,
                alarms_present: false,
                recurrence_present: false,
                url: None,
                backup_partial: false,
            })
            .collect();
        Ok(ReminderSnapshot {
            lists: vec![ReminderList {
                id: DEMO_LIST.into(),
                name: "Reminders".into(),
                writable: true,
            }],
            reminders,
        })
    }

    fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
        Ok(ApplyResult {
            written: changes
                .iter()
                .enumerate()
                .map(|(index, change)| demo_written(index, change))
                .collect(),
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
            priority,
        } => Some(StoredReminder {
            id: format!("demo-added-{index}"),
            list_id: list_id.clone(),
            title: title.clone(),
            notes: notes.clone(),
            due: *due,
            due_components: None,
            priority: *priority,
            completed: false,
            modified: None,
            alarms_present: false,
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
            .map(|(id, title, start, all_day, _)| Event {
                id: id.into(),
                title: title.into(),
                start,
                all_day,
            })
            .collect())
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
        run(
            cli,
            &crate::domain::Config::default(),
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
        TITLE                DUE         PRIORITY
        Review pull request  2026-10-26  high
        Pay rent             2026-10-25  medium
        Renew passport       2026-11-09  low
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
}

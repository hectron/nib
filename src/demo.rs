use jiff::civil::Date;
use jiff::{ToSpan, Zoned};

use crate::backend::{
    BackendError, Backends, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery,
    Notes, Priority, Reminder, ReminderChange, ReminderList, ReminderSnapshot, Reminders,
    StoredReminder,
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
            .map_err(|e| failed(app, e))
    }

    fn at(&self, offset: i32, hour: i8, minute: i8) -> Result<Zoned, BackendError> {
        self.day(offset, "Calendar")?
            .at(hour, minute, 0, 0)
            .to_zoned(self.0.time_zone().clone())
            .map_err(|e| failed("Calendar", e))
    }
}

fn failed(app: &'static str, error: jiff::Error) -> BackendError {
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
            })
            .collect())
    }
}

impl Reminders for Demo {
    fn add(&self, reminder: NewReminder) -> Result<Reminder, BackendError> {
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
            reminder("reminder-2", "Pay rent", Some(3), Some(Priority::Medium)),
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
                priority: r.priority,
                modified: None,
                has_alarms: false,
                has_recurrence: false,
                has_url: false,
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

    fn apply(&self, _: &[ReminderChange]) -> Result<(), BackendError> {
        Ok(())
    }
}

impl Calendar for Demo {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
        let events = [
            ("event-1", "Book club", self.at(-1, 18, 0)?, false),
            ("event-2", "Standup", self.at(0, 9, 30)?, false),
            ("event-3", "Design review", self.at(0, 14, 0)?, false),
            ("event-4", "Lunch with Sam", self.at(2, 12, 0)?, false),
            ("event-5", "Team offsite", self.at(3, 0, 0)?, true),
            ("event-6", "Sprint planning", self.at(5, 10, 0)?, false),
            ("event-7", "Retro", self.at(9, 16, 0)?, false),
        ];
        Ok(events
            .into_iter()
            .filter(|(_, _, start, _)| query.range.contains(start))
            .map(|(id, title, start, all_day)| Event {
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
    use crate::cli::Cli;
    use crate::commands::run;
    use crate::host::Scripted;
    use crate::output::Style;

    fn nib(args: &[&str]) -> String {
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let cli = Cli::try_parse_from(["nib"].iter().chain(args)).unwrap();
        run(
            cli,
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
        Pay rent             2026-10-29  medium
        Renew passport       2026-11-09  low
        Buy groceries
        ");
    }

    #[test]
    fn notes_filter_by_folder() {
        assert_snapshot!(nib(&["n", "-f", "Personal"]), @r"
        TITLE       FOLDER
        Groceries   Personal
        Book ideas  Personal
        ");
    }
}

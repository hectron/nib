use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use jiff::civil::Date;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore, EKReminder};
use objc2_foundation::{NSArray, NSDateComponentUndefined, NSError, NSInteger};

use super::{BackendError, NewReminder, Priority, Reminder, Reminders, terminal_name};

const REMINDERS: &str = "Reminders";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EventKitReminders;

impl Reminders for EventKitReminders {
    fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
        Err(BackendError::NotImplemented { app: REMINDERS })
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let predicate = unsafe {
                store.predicateForIncompleteRemindersWithDueDateStarting_ending_calendars(
                    None, None, None,
                )
            };
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |found: *mut NSArray<EKReminder>| {
                let reminders = unsafe { found.as_ref() }
                    .map(|found| found.iter().map(|r| to_reminder(&r)).collect::<Vec<_>>());
                let _ = tx.send(reminders);
            });
            let _request =
                unsafe { store.fetchRemindersMatchingPredicate_completion(&predicate, &block) };
            let mut reminders = rx
                .recv_timeout(FETCH_TIMEOUT)
                .map_err(|_| failed(REMINDERS, "timed out waiting for reminders"))?
                .ok_or_else(|| failed(REMINDERS, "the reminder query returned nothing"))?;
            reminders.sort_by(|a, b| {
                (a.due.is_none(), a.due, &a.title).cmp(&(b.due.is_none(), b.due, &b.title))
            });
            Ok(reminders)
        })
    }
}

fn authorized_store(
    entity: EKEntityType,
    app: &'static str,
) -> Result<Retained<EKEventStore>, BackendError> {
    let status = unsafe { EKEventStore::authorizationStatusForEntityType(entity) };
    if status == EKAuthorizationStatus::FullAccess
        || (status == EKAuthorizationStatus::NotDetermined && request_access(app)?)
    {
        return Ok(unsafe { EKEventStore::new() });
    }
    Err(BackendError::PermissionDenied {
        app,
        terminal: terminal_name(std::env::var("TERM_PROGRAM").ok().as_deref()),
    })
}

fn request_access(app: &'static str) -> Result<bool, BackendError> {
    eprintln!("Waiting for macOS to grant access to {app}...");
    let store = unsafe { EKEventStore::new() };
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, _: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block)) };
    rx.recv()
        .map_err(|_| failed(app, "the permission request never answered"))
}

fn to_reminder(reminder: &EKReminder) -> Reminder {
    unsafe {
        Reminder {
            id: reminder.calendarItemIdentifier().to_string(),
            title: reminder.title().to_string(),
            due: reminder
                .dueDateComponents()
                .and_then(|parts| date_from_parts(parts.year(), parts.month(), parts.day())),
            priority: priority_from_eventkit(reminder.priority()),
        }
    }
}

fn priority_from_eventkit(value: usize) -> Option<Priority> {
    match value {
        1..=4 => Some(Priority::High),
        5 => Some(Priority::Medium),
        6..=9 => Some(Priority::Low),
        _ => None,
    }
}

fn date_from_parts(year: NSInteger, month: NSInteger, day: NSInteger) -> Option<Date> {
    if [year, month, day].contains(&NSDateComponentUndefined) {
        return None;
    }
    Date::new(
        year.try_into().ok()?,
        month.try_into().ok()?,
        day.try_into().ok()?,
    )
    .ok()
}

fn failed(app: &'static str, message: &str) -> BackendError {
    BackendError::Failed {
        app,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn priorities_read_as_ranges() {
        assert_eq!(priority_from_eventkit(0), None);
        assert_eq!(priority_from_eventkit(1), Some(Priority::High));
        assert_eq!(priority_from_eventkit(4), Some(Priority::High));
        assert_eq!(priority_from_eventkit(5), Some(Priority::Medium));
        assert_eq!(priority_from_eventkit(6), Some(Priority::Low));
        assert_eq!(priority_from_eventkit(9), Some(Priority::Low));
        assert_eq!(priority_from_eventkit(10), None);
    }

    #[test]
    fn due_dates_need_every_part() {
        assert_eq!(date_from_parts(2026, 10, 27), Some(date(2026, 10, 27)));
        assert_eq!(date_from_parts(NSDateComponentUndefined, 10, 27), None);
        assert_eq!(date_from_parts(2026, NSDateComponentUndefined, 27), None);
        assert_eq!(date_from_parts(2026, 2, 30), None);
    }

    #[test]
    #[ignore = "reads your real Reminders; run with --ignored from a terminal that has access"]
    fn lists_real_reminders() {
        EventKitReminders.list().unwrap();
    }
}

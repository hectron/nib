use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use jiff::civil::Date;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore, EKReminder};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarIdentifierGregorian, NSDateComponentUndefined, NSDateComponents,
    NSError, NSInteger, NSString,
};

use super::{BackendError, NewReminder, Priority, Reminder, Reminders, terminal_name};

const REMINDERS: &str = "Reminders";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EventKitReminders;

impl Reminders for EventKitReminders {
    fn add(&self, new: NewReminder) -> Result<Reminder, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let list = unsafe { store.defaultCalendarForNewReminders() }
                .ok_or_else(|| failed(REMINDERS, "there is no default list for new reminders"))?;
            unsafe {
                let reminder = EKReminder::reminderWithEventStore(&store);
                reminder.setTitle(Some(&NSString::from_str(&new.title)));
                reminder.setCalendar(Some(&list));
                reminder.setPriority(priority_to_eventkit(new.priority));
                if let Some(due) = new.due {
                    reminder.setDueDateComponents(Some(&date_components(due)));
                }
                store
                    .saveReminder_commit_error(&reminder, true)
                    .map_err(|e| failed(REMINDERS, &e.localizedDescription().to_string()))?;
                Ok(to_reminder(&reminder))
            }
        })
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

fn priority_to_eventkit(priority: Option<Priority>) -> usize {
    match priority {
        Some(Priority::High) => 1,
        Some(Priority::Medium) => 5,
        Some(Priority::Low) => 9,
        None => 0,
    }
}

fn date_components(date: Date) -> Retained<NSDateComponents> {
    let parts = NSDateComponents::new();
    parts.setCalendar(
        NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian }).as_deref(),
    );
    parts.setYear(date.year().into());
    parts.setMonth(date.month().into());
    parts.setDay(date.day().into());
    parts
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
    fn priorities_round_trip() {
        for priority in [Priority::High, Priority::Medium, Priority::Low] {
            assert_eq!(
                priority_from_eventkit(priority_to_eventkit(Some(priority))),
                Some(priority)
            );
        }
        assert_eq!(priority_to_eventkit(None), 0);
    }

    #[test]
    fn due_dates_round_trip_through_gregorian_components() {
        let parts = date_components(date(2026, 2, 28));
        assert_eq!(
            date_from_parts(parts.year(), parts.month(), parts.day()),
            Some(date(2026, 2, 28))
        );
        let calendar = parts.calendar().unwrap();
        assert_eq!(
            calendar.calendarIdentifier().to_string(),
            unsafe { NSCalendarIdentifierGregorian }.to_string()
        );
    }

    #[test]
    #[ignore = "writes to your real Reminders; set NIB_LIVE_WRITE=1 and run with --ignored"]
    fn adds_and_removes_a_real_reminder() {
        if std::env::var_os("NIB_LIVE_WRITE").is_none() {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Reminders");
            return;
        }
        let added = EventKitReminders
            .add(NewReminder {
                title: "nib live test (safe to delete)".into(),
                due: Some(date(2030, 1, 15)),
                priority: Some(Priority::Low),
            })
            .unwrap();
        assert_eq!(added.due, Some(date(2030, 1, 15)));
        assert_eq!(added.priority, Some(Priority::Low));
        let store = authorized_store(EKEntityType::Reminder, REMINDERS).unwrap();
        let reminder = unsafe { store.calendarItemWithIdentifier(&NSString::from_str(&added.id)) }
            .unwrap()
            .downcast::<EKReminder>()
            .unwrap();
        unsafe { store.removeReminder_commit_error(&reminder, true) }.unwrap();
    }

    #[test]
    #[ignore = "reads your real Reminders; run with --ignored from a terminal that has access"]
    fn lists_real_reminders() {
        EventKitReminders.list().unwrap();
    }
}

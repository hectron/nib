use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStore, EKReminder,
};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSDateComponentUndefined,
    NSDateComponents, NSError, NSInteger, NSPredicate, NSString,
};

use crate::app::markdown::normalize_title;
use crate::domain::{
    BackendError, Calendar, Event, EventQuery, FieldEdit, NewReminder, Priority, Reminder,
    ReminderChange, ReminderDueComponents, ReminderEdit, ReminderList, ReminderSnapshot, Reminders,
    StoredReminder, terminal_name,
};

const REMINDERS: &str = "Reminders";
const CALENDARS: &str = "Calendars";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(300);

pub struct EventKitCalendar;

impl Calendar for EventKitCalendar {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Event, CALENDARS)?;
            let zone = query.range.start.time_zone().clone();
            let start = NSDate::dateWithTimeIntervalSince1970(epoch_seconds(&query.range.start));
            let end = NSDate::dateWithTimeIntervalSince1970(epoch_seconds(&query.range.end));
            let calendars = event_calendars(&store, &query.calendars)?;
            let chosen = (!calendars.is_empty()).then(|| NSArray::from_retained_slice(&calendars));
            let mut events = matching_events(&store, &start, &end, chosen.as_deref(), &zone);
            events.sort_by(|a, b| (a.start.timestamp(), &a.id).cmp(&(b.start.timestamp(), &b.id)));
            Ok(events)
        })
    }
}

pub struct EventKitReminders;

impl Reminders for EventKitReminders {
    fn add(&self, new: NewReminder) -> Result<Reminder, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let list = match new.list.as_deref() {
                Some(name) => writable_reminder_calendar_named(&store, name)?,
                None => default_reminder_calendar(&store).ok_or_else(|| {
                    failed(REMINDERS, "there is no default list for new reminders")
                })?,
            };
            add_reminder(&store, &list, &new)
        })
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let predicate = incomplete_reminders_predicate(&store, None);
            let mut reminders = fetch_reminders(&store, &predicate, to_reminder)?;
            reminders.sort_by(|a, b| {
                (a.due.is_none(), a.due, &a.title).cmp(&(b.due.is_none(), b.due, &b.title))
            });
            Ok(reminders)
        })
    }

    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let calendars: Vec<Retained<EKCalendar>> = calendars(&store, EKEntityType::Reminder)
                .into_iter()
                .filter(|calendar| list.is_none_or(|name| calendar_title(calendar) == name))
                .collect();
            if let Some(name) = list
                && calendars.is_empty()
            {
                return Err(BackendError::NotFound {
                    kind: "reminder list",
                    name: name.into(),
                });
            }
            let lists = calendars
                .iter()
                .map(|calendar| reminder_list(calendar))
                .collect();
            if calendars.is_empty() {
                return Ok(ReminderSnapshot::default());
            }
            let chosen = NSArray::from_retained_slice(&calendars);
            let predicate = incomplete_reminders_predicate(&store, Some(&chosen));
            let mut reminders = fetch_reminders(&store, &predicate, to_stored)?;
            reminders.sort_by(|a, b| {
                (a.due.is_none(), a.due, &a.title).cmp(&(b.due.is_none(), b.due, &b.title))
            });
            Ok(ReminderSnapshot { lists, reminders })
        })
    }

    fn apply(&self, changes: &[ReminderChange]) -> Result<(), BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            if let Err(e) = stage(&store, changes) {
                reset_store(&store);
                return Err(e);
            }
            commit_store(&store).map_err(|e| {
                reset_store(&store);
                failed(
                    REMINDERS,
                    &format!("nothing was saved: {}", e.localizedDescription()),
                )
            })
        })
    }
}

fn matching_events(
    store: &EKEventStore,
    start: &NSDate,
    end: &NSDate,
    calendars: Option<&NSArray<EKCalendar>>,
    zone: &TimeZone,
) -> Vec<Event> {
    // SAFETY: The dates, calendars, and retained store are valid Objective-C objects for the
    // duration of both synchronous EventKit calls, and EventKit returns EKEvent instances.
    let found = unsafe {
        let predicate =
            store.predicateForEventsWithStartDate_endDate_calendars(start, end, calendars);
        store.eventsMatchingPredicate(&predicate)
    };
    found
        .iter()
        .filter_map(|event| to_event(&event, zone))
        .collect()
}

fn add_reminder(
    store: &EKEventStore,
    list: &EKCalendar,
    new: &NewReminder,
) -> Result<Reminder, BackendError> {
    // SAFETY: The retained store and list belong to EventKit, all NSString arguments live through
    // their setter calls, and the new reminder is saved before leaving the autorelease pool.
    unsafe {
        let reminder = EKReminder::reminderWithEventStore(store);
        reminder.setTitle(Some(&NSString::from_str(&new.title)));
        reminder.setCalendar(Some(list));
        reminder.setPriority(priority_to_eventkit(new.priority));
        set_due(&reminder, new.due);
        store
            .saveReminder_commit_error(&reminder, true)
            .map_err(|error| failed(REMINDERS, &error.localizedDescription().to_string()))?;
        Ok(to_reminder(&reminder))
    }
}

fn incomplete_reminders_predicate(
    store: &EKEventStore,
    calendars: Option<&NSArray<EKCalendar>>,
) -> Retained<NSPredicate> {
    // SAFETY: The retained store and optional retained calendar array are valid for this
    // synchronous predicate construction, and both date bounds are intentionally unbounded.
    unsafe {
        store.predicateForIncompleteRemindersWithDueDateStarting_ending_calendars(
            None, None, calendars,
        )
    }
}

fn fetch_reminders<T: Send + 'static>(
    store: &EKEventStore,
    predicate: &NSPredicate,
    convert: fn(&EKReminder) -> T,
) -> Result<Vec<T>, BackendError> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |found: *mut NSArray<EKReminder>| {
        // SAFETY: EventKit invokes this callback with either null or a valid NSArray<EKReminder>
        // for the callback's duration; every element is converted to owned Rust data here.
        let values = unsafe { found.as_ref() }
            .map(|found| found.iter().map(|reminder| convert(&reminder)).collect());
        let _ = tx.send(values);
    });
    // SAFETY: The retained store and predicate remain alive until after the receive, and the block
    // is retained by both this stack frame and the returned EventKit request token.
    let _request = unsafe { store.fetchRemindersMatchingPredicate_completion(predicate, &block) };
    rx.recv_timeout(FETCH_TIMEOUT)
        .map_err(|_| failed(REMINDERS, "timed out waiting for reminders"))?
        .ok_or_else(|| failed(REMINDERS, "the reminder query returned nothing"))
}

fn calendars(store: &EKEventStore, entity: EKEntityType) -> Vec<Retained<EKCalendar>> {
    // SAFETY: The retained store is valid for this synchronous query and EventKit returns an array
    // whose elements are EKCalendar objects retained while collected.
    unsafe { store.calendarsForEntityType(entity) }
        .iter()
        .collect()
}

fn default_reminder_calendar(store: &EKEventStore) -> Option<Retained<EKCalendar>> {
    // SAFETY: The retained store is valid and EventKit returns either nil or a retained EKCalendar.
    unsafe { store.defaultCalendarForNewReminders() }
}

fn calendar_title(calendar: &EKCalendar) -> String {
    // SAFETY: A typed retained EKCalendar guarantees the title selector and returned NSString type.
    unsafe { calendar.title() }.to_string()
}

fn calendar_id(calendar: &EKCalendar) -> String {
    // SAFETY: A typed retained EKCalendar guarantees the identifier selector and NSString result.
    unsafe { calendar.calendarIdentifier() }.to_string()
}

fn calendar_is_writable(calendar: &EKCalendar) -> bool {
    // SAFETY: A typed retained EKCalendar guarantees the content-modification selector.
    unsafe { calendar.allowsContentModifications() }
}

fn reminder_list(calendar: &EKCalendar) -> ReminderList {
    ReminderList {
        id: calendar_id(calendar),
        name: calendar_title(calendar),
        writable: calendar_is_writable(calendar),
    }
}

fn reset_store(store: &EKEventStore) {
    // SAFETY: The retained store is valid and reset has no borrowed Objective-C return value.
    unsafe { store.reset() };
}

fn commit_store(store: &EKEventStore) -> Result<(), Retained<NSError>> {
    // SAFETY: The retained store is valid and all staged objects remain owned by EventKit.
    unsafe { store.commit() }
}

fn event_calendars(
    store: &EKEventStore,
    names: &[String],
) -> Result<Vec<Retained<EKCalendar>>, BackendError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let available = calendars(store, EKEntityType::Event);
    for name in names {
        if !available
            .iter()
            .any(|calendar| calendar_title(calendar) == *name)
        {
            return Err(BackendError::NotFound {
                kind: "calendar",
                name: name.clone(),
            });
        }
    }
    Ok(available
        .into_iter()
        .filter(|calendar| {
            let title = calendar_title(calendar);
            names.contains(&title)
        })
        .collect())
}

fn writable_reminder_calendar_named(
    store: &EKEventStore,
    name: &str,
) -> Result<Retained<EKCalendar>, BackendError> {
    let mut matches = calendars(store, EKEntityType::Reminder)
        .into_iter()
        .filter(|calendar| calendar_is_writable(calendar) && calendar_title(calendar) == name)
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Err(BackendError::NotFound {
            kind: "writable reminder list",
            name: name.into(),
        }),
        1 => Ok(matches
            .pop()
            .expect("a one-element vector must contain one calendar")),
        _ => Err(BackendError::Ambiguous {
            kind: "writable reminder list",
            name: name.into(),
        }),
    }
}

fn stage(store: &EKEventStore, changes: &[ReminderChange]) -> Result<(), BackendError> {
    let mut current = Vec::with_capacity(changes.len());
    let mut conflicts = Vec::new();
    for change in changes {
        match change {
            ReminderChange::Add { .. } => current.push(None),
            ReminderChange::Update { before, .. } | ReminderChange::Delete { before } => {
                match find_reminder(store, &before.id) {
                    Some(reminder) if to_stored(&reminder) == *before => {
                        current.push(Some(reminder));
                    }
                    _ => {
                        conflicts.push(format!("\"{}\"", normalize_title(&before.title)));
                        current.push(None);
                    }
                }
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(BackendError::Conflict { titles: conflicts });
    }
    for (change, reminder) in changes.iter().zip(current) {
        match (change, reminder) {
            (
                ReminderChange::Add {
                    list_id,
                    title,
                    notes,
                    due,
                    priority,
                },
                _,
            ) => stage_add(store, list_id, title, notes.as_deref(), *due, *priority)?,
            (ReminderChange::Update { before, edit }, Some(reminder)) => {
                update(store, &reminder, edit)?;
                save(store, &reminder, &before.title)?;
            }
            (ReminderChange::Delete { before }, Some(reminder)) => {
                remove_reminder(store, &reminder, false).map_err(|e| {
                    failed(
                        REMINDERS,
                        &format!(
                            "\"{}\" couldn't be deleted: {}",
                            normalize_title(&before.title),
                            e.localizedDescription()
                        ),
                    )
                })?;
            }
            _ => unreachable!("conflicts were checked above"),
        }
    }
    Ok(())
}

fn stage_add(
    store: &EKEventStore,
    list_id: &str,
    title: &str,
    notes: Option<&str>,
    due: Option<Date>,
    priority: Option<Priority>,
) -> Result<(), BackendError> {
    let list = writable_calendar(store, list_id)?;
    // SAFETY: The retained EventKit store and calendar are valid, temporary NSString values live
    // through each setter call, and the reminder is staged before any object can be released.
    unsafe {
        let reminder = EKReminder::reminderWithEventStore(store);
        reminder.setCalendar(Some(&list));
        reminder.setTitle(Some(&NSString::from_str(title)));
        reminder.setNotes(notes.map(NSString::from_str).as_deref());
        reminder.setPriority(priority_to_eventkit(priority));
        set_due(&reminder, due);
        save(store, &reminder, title)
    }
}

fn update(
    store: &EKEventStore,
    reminder: &EKReminder,
    edit: &ReminderEdit,
) -> Result<(), BackendError> {
    // SAFETY: The typed reminder and store are retained for every setter call, temporary NSString
    // values outlive their calls, and any destination calendar is retained before assignment.
    unsafe {
        if let Some(title) = &edit.title {
            reminder.setTitle(Some(&NSString::from_str(title)));
        }
        match &edit.notes {
            FieldEdit::Set(notes) => reminder.setNotes(Some(&NSString::from_str(notes))),
            FieldEdit::Remove => reminder.setNotes(None),
            FieldEdit::Unchanged => {}
        }
        match edit.due {
            FieldEdit::Set(due) => set_due(reminder, Some(due)),
            FieldEdit::Remove => set_due(reminder, None),
            FieldEdit::Unchanged => {}
        }
        match edit.priority {
            FieldEdit::Set(priority) => {
                reminder.setPriority(priority_to_eventkit(Some(priority)));
            }
            FieldEdit::Remove => reminder.setPriority(priority_to_eventkit(None)),
            FieldEdit::Unchanged => {}
        }
        if let Some(list_id) = &edit.list_id {
            reminder.setCalendar(Some(&*writable_calendar(store, list_id)?));
        }
        if edit.complete {
            reminder.setCompleted(true);
        }
    }
    Ok(())
}

fn set_due(reminder: &EKReminder, due: Option<Date>) {
    // SAFETY: The typed reminder is retained for all accesses, and the newly built or retained
    // NSDateComponents object remains alive through assignment back to the reminder.
    unsafe {
        let Some(date) = due else {
            reminder.setDueDateComponents(None);
            return;
        };
        let parts = reminder
            .dueDateComponents()
            .unwrap_or_else(|| date_components(date));
        if parts.calendar().is_none() {
            parts.setCalendar(gregorian_calendar().as_deref());
        }
        parts.setYear(date.year().into());
        parts.setMonth(date.month().into());
        parts.setDay(date.day().into());
        reminder.setDueDateComponents(Some(&parts));
    }
}

fn save(store: &EKEventStore, reminder: &EKReminder, title: &str) -> Result<(), BackendError> {
    // SAFETY: The typed store and reminder are retained, and EventKit owns the staged reminder for
    // the duration of the synchronous save operation.
    unsafe { store.saveReminder_commit_error(reminder, false) }.map_err(|e| {
        failed(
            REMINDERS,
            &format!(
                "\"{}\" couldn't be saved: {}",
                normalize_title(title),
                e.localizedDescription()
            ),
        )
    })
}

fn find_reminder(store: &EKEventStore, id: &str) -> Option<Retained<EKReminder>> {
    // SAFETY: The typed store is retained, the temporary identifier lives through the lookup, and
    // downcast verifies the returned calendar item's Objective-C class before use.
    unsafe { store.calendarItemWithIdentifier(&NSString::from_str(id)) }?
        .downcast::<EKReminder>()
        .ok()
}

fn writable_calendar(store: &EKEventStore, id: &str) -> Result<Retained<EKCalendar>, BackendError> {
    // SAFETY: The typed store is retained and the temporary identifier lives through this
    // synchronous lookup; EventKit returns either nil or an EKCalendar.
    let calendar =
        unsafe { store.calendarWithIdentifier(&NSString::from_str(id)) }.ok_or_else(|| {
            BackendError::NotFound {
                kind: "reminder list",
                name: id.into(),
            }
        })?;
    if calendar_is_writable(&calendar) {
        Ok(calendar)
    } else {
        Err(failed(
            REMINDERS,
            &format!("\"{}\" is read-only", calendar_title(&calendar)),
        ))
    }
}

fn remove_reminder(
    store: &EKEventStore,
    reminder: &EKReminder,
    commit: bool,
) -> Result<(), Retained<NSError>> {
    // SAFETY: The typed store and reminder are retained for the synchronous removal call.
    unsafe { store.removeReminder_commit_error(reminder, commit) }
}

fn to_stored(reminder: &EKReminder) -> StoredReminder {
    // SAFETY: Every selector is invoked on a typed retained EKReminder and all Objective-C values
    // are copied into owned Rust values before this function returns.
    unsafe {
        let due = reminder.dueDateComponents();
        let url_object = reminder.URL();
        let url = url_object
            .as_ref()
            .and_then(|value| value.absoluteString())
            .map(|value| value.to_string());
        StoredReminder {
            id: reminder.calendarItemIdentifier().to_string(),
            list_id: reminder
                .calendar()
                .map(|c| c.calendarIdentifier().to_string())
                .unwrap_or_default(),
            title: reminder.title().to_string(),
            notes: reminder.notes().map(|n| n.to_string()),
            due: due
                .as_ref()
                .and_then(|parts| date_from_parts(parts.year(), parts.month(), parts.day())),
            due_components: due.as_deref().map(reminder_due_components),
            priority: priority_from_eventkit(reminder.priority()),
            modified: reminder
                .lastModifiedDate()
                .and_then(|d| timestamp_from_epoch(d.timeIntervalSince1970())),
            alarms_present: reminder.hasAlarms(),
            recurrence_present: reminder.hasRecurrenceRules(),
            backup_partial: url_object.is_some() && url.is_none(),
            url,
        }
    }
}

fn reminder_due_components(parts: &NSDateComponents) -> ReminderDueComponents {
    ReminderDueComponents {
        calendar: parts
            .calendar()
            .map(|calendar| calendar.calendarIdentifier().to_string()),
        time_zone: parts.timeZone().map(|zone| zone.name().to_string()),
        era: component(parts.era()),
        year: component(parts.year()),
        month: component(parts.month()),
        day: component(parts.day()),
        hour: component(parts.hour()),
        minute: component(parts.minute()),
        second: component(parts.second()),
        nanosecond: component(parts.nanosecond()),
        weekday: component(parts.weekday()),
        weekday_ordinal: component(parts.weekdayOrdinal()),
        quarter: component(parts.quarter()),
        week_of_month: component(parts.weekOfMonth()),
        week_of_year: component(parts.weekOfYear()),
        year_for_week_of_year: component(parts.yearForWeekOfYear()),
        day_of_year: component(parts.dayOfYear()),
        is_leap_month: parts.isLeapMonth(),
    }
}

fn component(value: NSInteger) -> Option<i64> {
    (value != NSDateComponentUndefined)
        .then_some(value)
        .and_then(|value| i64::try_from(value).ok())
}

fn timestamp_from_epoch(seconds: f64) -> Option<Timestamp> {
    jiff::SignedDuration::try_from_secs_f64(seconds)
        .ok()
        .and_then(|d| Timestamp::from_duration(d).ok())
}

fn authorized_store(
    entity: EKEntityType,
    app: &'static str,
) -> Result<Retained<EKEventStore>, BackendError> {
    let status = authorization_status(entity);
    if status == EKAuthorizationStatus::FullAccess
        || (status == EKAuthorizationStatus::NotDetermined && request_access(entity, app)?)
    {
        return Ok(new_event_store());
    }
    Err(BackendError::PermissionDenied {
        app,
        terminal: terminal_name(std::env::var("TERM_PROGRAM").ok().as_deref()),
    })
}

fn authorization_status(entity: EKEntityType) -> EKAuthorizationStatus {
    // SAFETY: EKEntityType values come from EventKit's closed set and this class query has no
    // borrowed return value or caller-managed memory.
    unsafe { EKEventStore::authorizationStatusForEntityType(entity) }
}

fn new_event_store() -> Retained<EKEventStore> {
    // SAFETY: NSObject-style `new` returns a retained EKEventStore with no caller preconditions.
    unsafe { EKEventStore::new() }
}

fn request_access(entity: EKEntityType, app: &'static str) -> Result<bool, BackendError> {
    eprintln!("Waiting for macOS to grant access to {app}...");
    let store = new_event_store();
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        // SAFETY: EventKit passes either null or a valid NSError pointer for the duration of the
        // callback; the localized description is copied to an owned String before returning.
        let result = unsafe { error.as_ref() }.map_or_else(
            || Ok(granted.as_bool()),
            |error| Err(error.localizedDescription().to_string()),
        );
        let _ = tx.send(result);
    });
    // SAFETY: The retained store and copied block remain alive until the callback sends its owned
    // result, and the selected request method matches the requested EventKit entity type.
    unsafe {
        if entity == EKEntityType::Event {
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
        } else {
            store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block));
        }
    }
    rx.recv_timeout(PROMPT_TIMEOUT)
        .map_err(|_| failed(app, "timed out waiting for the macOS permission prompt"))
        .and_then(|result| result.map_err(|message| failed(app, &message)))
}

fn to_event(event: &EKEvent, zone: &TimeZone) -> Option<Event> {
    // SAFETY: Every selector is invoked on a typed retained EKEvent and all returned values are
    // converted to owned Rust data before this function returns.
    unsafe {
        let start = zoned_from_epoch(event.startDate().timeIntervalSince1970(), zone)?;
        let identifier = event
            .eventIdentifier()
            .unwrap_or_else(|| event.calendarItemIdentifier());
        Some(Event {
            id: event_id(&identifier.to_string(), start.timestamp()),
            title: event.title().to_string(),
            start,
            all_day: event.isAllDay(),
        })
    }
}

fn event_id(identifier: &str, start: Timestamp) -> String {
    format!("{identifier}@{start}")
}

fn epoch_seconds(time: &Zoned) -> f64 {
    time.timestamp().as_duration().as_secs_f64()
}

fn zoned_from_epoch(seconds: f64, zone: &TimeZone) -> Option<Zoned> {
    timestamp_from_epoch(seconds.floor()).map(|timestamp| timestamp.to_zoned(zone.clone()))
}

fn to_reminder(reminder: &EKReminder) -> Reminder {
    // SAFETY: Every selector is invoked on a typed retained EKReminder and all returned values are
    // converted to owned Rust data before this function returns.
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
    parts.setCalendar(gregorian_calendar().as_deref());
    parts.setYear(date.year().into());
    parts.setMonth(date.month().into());
    parts.setDay(date.day().into());
    parts
}

fn gregorian_calendar() -> Option<Retained<NSCalendar>> {
    // SAFETY: Foundation exposes this as a process-lifetime calendar identifier constant, and the
    // constructor copies or retains it during the synchronous call.
    NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
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
    use crate::app::markdown;

    struct LiveReminderCleanup {
        id: Option<String>,
        list_id: String,
        list_name: String,
        title: String,
    }

    impl Drop for LiveReminderCleanup {
        fn drop(&mut self) {
            let id = self.id.clone().or_else(|| {
                EventKitReminders
                    .snapshot(Some(&self.list_name))
                    .ok()?
                    .reminders
                    .into_iter()
                    .find(|reminder| {
                        reminder.list_id == self.list_id && reminder.title == self.title
                    })
                    .map(|reminder| reminder.id)
            });
            let Some(id) = id else {
                eprintln!("cleanup failed: couldn't find the live test reminder");
                return;
            };
            autoreleasepool(|_| {
                let Ok(store) = authorized_store(EKEntityType::Reminder, REMINDERS) else {
                    eprintln!("cleanup failed: couldn't open Reminders");
                    return;
                };
                let Some(reminder) = find_reminder(&store, &id) else {
                    return;
                };
                if let Err(error) = remove_reminder(&store, &reminder, true) {
                    eprintln!("cleanup failed: {}", error.localizedDescription());
                }
            });
        }
    }

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
    fn event_ids_pair_the_identifier_with_the_utc_start() {
        let start = date(2026, 10, 27)
            .at(10, 0, 0, 0)
            .to_zoned(TimeZone::fixed(jiff::tz::offset(-4)))
            .unwrap();
        assert_eq!(
            event_id("ABC123", start.timestamp()),
            "ABC123@2026-10-27T14:00:00Z"
        );
    }

    #[test]
    fn event_times_round_trip_through_epoch_seconds() {
        let zone = TimeZone::fixed(jiff::tz::offset(-4));
        let start = date(2026, 10, 27)
            .at(10, 30, 0, 0)
            .to_zoned(zone.clone())
            .unwrap();
        assert_eq!(
            zoned_from_epoch(epoch_seconds(&start), &zone),
            Some(start.clone())
        );
        assert_eq!(
            zoned_from_epoch(epoch_seconds(&start) + 0.75, &zone),
            Some(start)
        );
    }

    #[test]
    #[ignore = "reads your real calendars; run with --ignored from a terminal that has access"]
    fn lists_real_events() {
        let now = Timestamp::now().to_zoned(TimeZone::system());
        let start = now.start_of_day().unwrap();
        let end = start.checked_add(jiff::Span::new().days(7)).unwrap();
        EventKitCalendar
            .events(EventQuery {
                range: start..end,
                calendars: Vec::new(),
            })
            .unwrap();
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
        let gregorian =
            gregorian_calendar().expect("Foundation always provides Gregorian calendar");
        assert_eq!(
            calendar.calendarIdentifier().to_string(),
            gregorian.calendarIdentifier().to_string()
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
                list: None,
                due: Some(date(2030, 1, 15)),
                priority: Some(Priority::Low),
            })
            .unwrap();
        assert_eq!(added.due, Some(date(2030, 1, 15)));
        assert_eq!(added.priority, Some(Priority::Low));
        let store = authorized_store(EKEntityType::Reminder, REMINDERS).unwrap();
        let reminder = find_reminder(&store, &added.id).unwrap();
        remove_reminder(&store, &reminder, true).unwrap();
    }

    #[test]
    #[ignore = "writes to your real Reminders; set NIB_LIVE_WRITE=1 and run with --ignored"]
    fn edits_a_real_reminder_through_markdown() {
        if std::env::var_os("NIB_LIVE_WRITE").is_none() {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Reminders");
            return;
        }
        let store = authorized_store(EKEntityType::Reminder, REMINDERS).unwrap();
        let list =
            default_reminder_calendar(&store).expect("there is no default list for new reminders");
        let list_id = calendar_id(&list);
        let list_name = calendar_title(&list);
        let title = format!(
            "nib live test {} *bold* #tag \\ 📅 2030-01-01",
            std::process::id()
        );
        let notes = "first line  \n\n\tindented line   \nlast line  ";
        let mut cleanup = LiveReminderCleanup {
            id: None,
            list_id: list_id.clone(),
            list_name: list_name.clone(),
            title: title.clone(),
        };

        EventKitReminders
            .apply(&[ReminderChange::Add {
                list_id: list_id.clone(),
                title: title.clone(),
                notes: Some(notes.into()),
                due: None,
                priority: None,
            }])
            .unwrap();

        let snapshot = EventKitReminders.snapshot(Some(&list_name)).unwrap();
        let reminder = snapshot
            .reminders
            .iter()
            .find(|reminder| reminder.list_id == list_id && reminder.title == title)
            .unwrap();
        cleanup.id = Some(reminder.id.clone());
        let reminder_id = reminder.id.clone();
        let session = markdown::Session::new(&snapshot);
        let rendered = markdown::render(&session.document);
        let parsed = markdown::parse(&rendered).unwrap();
        assert_eq!(
            markdown::plan(&snapshot, &session, &parsed).unwrap(),
            markdown::Plan::default()
        );

        let short_id = session
            .document
            .sections
            .iter()
            .flat_map(|section| &section.items)
            .find(|item| item.title == title)
            .and_then(|item| item.id.as_deref())
            .unwrap();
        let id_marker = format!("🆔 {short_id}");
        let original_line = rendered
            .lines()
            .find(|line| line.starts_with("- [ ]") && line.ends_with(&id_marker))
            .unwrap();
        let edited_line = original_line
            .replacen("- [ ]", "- [x]", 1)
            .replacen("nib live test", "nib edited live test", 1)
            .replacen(
                &format!(" {id_marker}"),
                &format!(" ⏫ 📅 2030-02-03 {id_marker}"),
                1,
            );
        let edited = rendered.replacen(original_line, &edited_line, 1);
        let parsed = markdown::parse(&edited).unwrap();
        let plan = markdown::plan(&snapshot, &session, &parsed).unwrap();
        assert!(matches!(
            &plan.changes[..],
            [ReminderChange::Update { edit, .. }]
                if edit.title.as_deref()
                    == Some(title.replacen("nib live test", "nib edited live test", 1).as_str())
                    && edit.due == FieldEdit::Set(date(2030, 2, 3))
                    && edit.priority == FieldEdit::Set(Priority::High)
                    && edit.complete
        ));
        EventKitReminders.apply(&plan.changes).unwrap();

        let after = EventKitReminders.snapshot(Some(&list_name)).unwrap();
        assert!(
            after
                .reminders
                .iter()
                .all(|reminder| reminder.id != reminder_id)
        );
    }

    #[test]
    #[ignore = "reads your real Reminders; run with --ignored from a terminal that has access"]
    fn lists_real_reminders() {
        EventKitReminders.list().unwrap();
    }
}

use std::cmp::Ordering;
use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAuthorizationStatus, EKCalendar, EKCalendarType, EKEntityType, EKEvent, EKEventStore,
    EKReminder, EKSource, EKSourceType,
};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSDateComponentUndefined,
    NSDateComponents, NSError, NSInteger, NSPredicate, NSString, NSTimeZone, NSURL,
};

use crate::app::markdown::normalize_title;
use crate::domain::{
    ApplyResult, AutomationStatus, BackendError, Calendar, CalendarInfo, CalendarKind, Event,
    EventQuery, FieldEdit, NewReminder, NewResource, PermissionStatus, Priority, Reminder,
    ReminderChange, ReminderDueComponents, ReminderEdit, ReminderList, ReminderListInfo,
    ReminderSnapshot, Reminders, ResourceSelector, StoredReminder, terminal_name,
};
use crate::infra::completions::CacheFile;

const REMINDERS: &str = "Reminders";
const CALENDARS: &str = "Calendars";
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(300);
const TYPE_APPLICATION_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
const TYPE_WILDCARD: u32 = u32::from_be_bytes(*b"****");
const NOTES_BUNDLE_ID: &[u8] = b"com.apple.Notes";
const NO_ERROR: i32 = 0;
const PROCESS_NOT_FOUND: i32 = -600;
const EVENT_NOT_PERMITTED: i32 = -1743;
const EVENT_WOULD_REQUIRE_CONSENT: i32 = -1744;

// AEDataModel.h declares AEDesc under `#pragma pack(push, 2)`.
#[repr(C, packed(2))]
struct AppleEventDescriptor {
    descriptor_type: u32,
    data_handle: *mut *mut c_void,
}

const _: () = assert!(std::mem::size_of::<AppleEventDescriptor>() == 12);

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    #[link_name = "AECreateDesc"]
    unsafe fn ae_create_desc(
        type_code: u32,
        data: *const c_void,
        size: isize,
        result: *mut AppleEventDescriptor,
    ) -> i16;
    #[link_name = "AEDisposeDesc"]
    unsafe fn ae_dispose_desc(descriptor: *mut AppleEventDescriptor) -> i16;
    #[link_name = "AEDeterminePermissionToAutomateTarget"]
    unsafe fn ae_determine_permission(
        target: *const AppleEventDescriptor,
        event_class: u32,
        event_id: u32,
        ask_user_if_needed: u8,
    ) -> i32;
}

pub struct EventKitCalendar {
    cache: CacheFile,
}

impl EventKitCalendar {
    pub fn new(cache: CacheFile) -> Self {
        Self { cache }
    }

    #[cfg(test)]
    fn uncached() -> Self {
        Self::new(CacheFile::new(None, 0))
    }
}

impl Calendar for EventKitCalendar {
    fn events(&self, query: EventQuery) -> Result<Vec<Event>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Event, CALENDARS)?;
            let names = self
                .cache
                .is_enabled()
                .then(|| calendar_names(&store, EKEntityType::Event));
            let zone = query.range.start.time_zone().clone();
            let start = NSDate::dateWithTimeIntervalSince1970(epoch_seconds(&query.range.start));
            let end = NSDate::dateWithTimeIntervalSince1970(epoch_seconds(&query.range.end));
            let calendars = event_calendars(&store, &query.calendars)?;
            let chosen = (!calendars.is_empty()).then(|| NSArray::from_retained_slice(&calendars));
            let mut events = matching_events(&store, &start, &end, chosen.as_deref(), &zone);
            events.sort_by(|a, b| (a.start.timestamp(), &a.id).cmp(&(b.start.timestamp(), &b.id)));
            if let Some(names) = names {
                self.cache.update_calendars(names);
            }
            Ok(events)
        })
    }

    fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Event, CALENDARS)?;
            let default = default_event_calendar(&store).as_deref().map(calendar_id);
            let mut values = calendars(&store, EKEntityType::Event)
                .iter()
                .map(|calendar| calendar_info(calendar, default.as_deref()))
                .collect::<Vec<_>>();
            values.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
            self.cache
                .update_calendars(values.iter().map(|value| value.name.clone()).collect());
            Ok(values)
        })
    }

    fn create_calendar(&self, calendar: NewResource) -> Result<CalendarInfo, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Event, CALENDARS)?;
            let calendar = create_calendar_resource(
                &store,
                EKEntityType::Event,
                &calendar,
                "calendar",
                CALENDARS,
            )?;
            let default = default_event_calendar(&store).as_deref().map(calendar_id);
            let value = calendar_info(&calendar, default.as_deref());
            self.cache
                .update_calendars(calendar_names(&store, EKEntityType::Event));
            Ok(value)
        })
    }

    fn rename_calendar(
        &self,
        calendar: ResourceSelector,
        name: String,
    ) -> Result<CalendarInfo, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Event, CALENDARS)?;
            let calendar = rename_calendar_resource(
                &store,
                EKEntityType::Event,
                &calendar,
                &name,
                "calendar",
                CALENDARS,
            )?;
            let default = default_event_calendar(&store).as_deref().map(calendar_id);
            let value = calendar_info(&calendar, default.as_deref());
            self.cache
                .update_calendars(calendar_names(&store, EKEntityType::Event));
            Ok(value)
        })
    }
}

pub struct EventKitReminders {
    cache: CacheFile,
}

impl EventKitReminders {
    pub fn new(cache: CacheFile) -> Self {
        Self { cache }
    }

    #[cfg(test)]
    fn uncached() -> Self {
        Self::new(CacheFile::new(None, 0))
    }

    fn update_cache(&self, store: &EKEventStore) {
        if !self.cache.is_enabled() {
            return;
        }
        let (names, writable) = reminder_list_names(store);
        self.cache.update_reminder_lists(names, writable);
    }
}

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
            let reminder = add_reminder(&store, &list, &new)?;
            self.update_cache(&store);
            Ok(reminder)
        })
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let predicate = incomplete_reminders_predicate(&store, None);
            let mut reminders = fetch_reminders(&store, &predicate, to_reminder)?;
            reminders.sort_by(compare_reminders);
            self.update_cache(&store);
            Ok(reminders)
        })
    }

    fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let snapshot = reminder_snapshot(&store, list, false)?;
            self.update_cache(&store);
            Ok(snapshot)
        })
    }

    fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let snapshot = reminder_snapshot(&store, None, true)?;
            self.update_cache(&store);
            Ok(snapshot)
        })
    }

    fn lists(&self) -> Result<Vec<ReminderListInfo>, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let default = default_reminder_calendar(&store)
                .as_deref()
                .map(calendar_id);
            let mut values = calendars(&store, EKEntityType::Reminder)
                .iter()
                .map(|calendar| reminder_list_info(calendar, default.as_deref()))
                .collect::<Vec<_>>();
            values.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
            self.update_cache(&store);
            Ok(values)
        })
    }

    fn create_list(&self, list: NewResource) -> Result<ReminderListInfo, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let list = create_calendar_resource(
                &store,
                EKEntityType::Reminder,
                &list,
                "reminder list",
                REMINDERS,
            )?;
            let default = default_reminder_calendar(&store)
                .as_deref()
                .map(calendar_id);
            let value = reminder_list_info(&list, default.as_deref());
            self.update_cache(&store);
            Ok(value)
        })
    }

    fn rename_list(
        &self,
        list: ResourceSelector,
        name: String,
    ) -> Result<ReminderListInfo, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let list = rename_calendar_resource(
                &store,
                EKEntityType::Reminder,
                &list,
                &name,
                "reminder list",
                REMINDERS,
            )?;
            let default = default_reminder_calendar(&store)
                .as_deref()
                .map(calendar_id);
            let value = reminder_list_info(&list, default.as_deref());
            self.update_cache(&store);
            Ok(value)
        })
    }

    fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
        autoreleasepool(|_| {
            let store = authorized_store(EKEntityType::Reminder, REMINDERS)?;
            let staged = stage(&store, changes).inspect_err(|_| reset_store(&store))?;
            commit_store(&store).map_err(|e| {
                reset_store(&store);
                failed(
                    REMINDERS,
                    &format!("nothing was saved: {}", e.localizedDescription()),
                )
            })?;
            let mut written = Vec::with_capacity(staged.len());
            let mut unrecorded = Vec::with_capacity(staged.len());
            for reminder in staged {
                if let Some(reminder) = reminder.as_deref() {
                    let (stored, was_unrecorded) = stored_after_commit(reminder);
                    written.push(Some(stored));
                    unrecorded.push(was_unrecorded);
                } else {
                    written.push(None);
                    unrecorded.push(false);
                }
            }
            self.update_cache(&store);
            Ok(ApplyResult {
                written,
                unrecorded,
            })
        })
    }

    fn validate_time_zone(&self, name: &str) -> Result<(), BackendError> {
        autoreleasepool(|_| {
            NSTimeZone::timeZoneWithName(&NSString::from_str(name))
                .map(|_| ())
                .ok_or_else(|| failed(REMINDERS, &format!("time zone {name:?} isn't available")))
        })
    }
}

fn compare_reminders(a: &Reminder, b: &Reminder) -> Ordering {
    compare_due(
        a.due, a.due_time, &a.title, &a.id, b.due, b.due_time, &b.title, &b.id,
    )
}

fn compare_stored_reminders(a: &StoredReminder, b: &StoredReminder) -> Ordering {
    compare_due(
        a.due,
        a.due_components.as_ref().and_then(stored_due_time),
        &a.title,
        &a.id,
        b.due,
        b.due_components.as_ref().and_then(stored_due_time),
        &b.title,
        &b.id,
    )
}

#[allow(clippy::too_many_arguments)]
fn compare_due(
    a_date: Option<Date>,
    a_time: Option<Time>,
    a_title: &str,
    a_id: &str,
    b_date: Option<Date>,
    b_time: Option<Time>,
    b_title: &str,
    b_id: &str,
) -> Ordering {
    (
        a_date.is_none(),
        a_date,
        a_date.is_some() && a_time.is_some(),
        a_date.and(a_time),
        a_title,
        a_id,
    )
        .cmp(&(
            b_date.is_none(),
            b_date,
            b_date.is_some() && b_time.is_some(),
            b_date.and(b_time),
            b_title,
            b_id,
        ))
}

fn stored_after_commit(reminder: &EKReminder) -> (StoredReminder, bool) {
    // SAFETY: The typed reminder remains alive while EventKit synchronously refreshes it, and nib
    // only observes the Boolean result before copying fields from that same live object.
    let unrecorded = !unsafe { reminder.refresh() };
    (to_stored(reminder), unrecorded)
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
        set_due(
            &reminder,
            new.due,
            new.due_time,
            new.due_time_zone.as_deref(),
        )?;
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

fn all_reminders_predicate(
    store: &EKEventStore,
    calendars: Option<&NSArray<EKCalendar>>,
) -> Retained<NSPredicate> {
    // SAFETY: The retained store and optional retained calendar array are valid for this
    // synchronous predicate construction.
    unsafe { store.predicateForRemindersInCalendars(calendars) }
}

fn reminder_snapshot(
    store: &EKEventStore,
    list: Option<&str>,
    include_completed: bool,
) -> Result<ReminderSnapshot, BackendError> {
    let calendars: Vec<Retained<EKCalendar>> = calendars(store, EKEntityType::Reminder)
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
    let predicate = if include_completed {
        all_reminders_predicate(store, Some(&chosen))
    } else {
        incomplete_reminders_predicate(store, Some(&chosen))
    };
    let mut reminders = fetch_reminders(store, &predicate, to_stored)?;
    reminders.sort_by(compare_stored_reminders);
    Ok(ReminderSnapshot { lists, reminders })
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

fn calendar_names(store: &EKEventStore, entity: EKEntityType) -> Vec<String> {
    calendars(store, entity)
        .iter()
        .map(|calendar| calendar_title(calendar))
        .collect()
}

fn reminder_list_names(store: &EKEventStore) -> (Vec<String>, Vec<String>) {
    let lists = calendars(store, EKEntityType::Reminder);
    let names = lists
        .iter()
        .map(|calendar| calendar_title(calendar))
        .collect();
    let writable = lists
        .iter()
        .filter(|calendar| calendar_is_writable(calendar))
        .map(|calendar| calendar_title(calendar))
        .collect();
    (names, writable)
}

fn default_reminder_calendar(store: &EKEventStore) -> Option<Retained<EKCalendar>> {
    // SAFETY: The retained store is valid and EventKit returns either nil or a retained EKCalendar.
    unsafe { store.defaultCalendarForNewReminders() }
}

fn default_event_calendar(store: &EKEventStore) -> Option<Retained<EKCalendar>> {
    // SAFETY: The retained store is valid and EventKit returns either nil or a retained EKCalendar.
    unsafe { store.defaultCalendarForNewEvents() }
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

fn calendar_is_immutable(calendar: &EKCalendar) -> bool {
    // SAFETY: A typed retained EKCalendar guarantees the immutable selector.
    unsafe { calendar.isImmutable() }
}

fn calendar_kind(calendar: &EKCalendar) -> CalendarKind {
    // SAFETY: A typed retained EKCalendar guarantees the type and subscription selectors.
    let (kind, subscribed) = unsafe { (calendar.r#type(), calendar.isSubscribed()) };
    if subscribed {
        return CalendarKind::Subscription;
    }
    match kind {
        EKCalendarType::Local => CalendarKind::Local,
        EKCalendarType::CalDAV => CalendarKind::CalDav,
        EKCalendarType::Exchange => CalendarKind::Exchange,
        EKCalendarType::Subscription => CalendarKind::Subscription,
        EKCalendarType::Birthday => CalendarKind::Birthday,
        _ => CalendarKind::Unknown,
    }
}

fn calendar_source(calendar: &EKCalendar) -> (Option<String>, Option<String>) {
    // SAFETY: A typed retained EKCalendar guarantees the source selector, and a returned EKSource
    // guarantees its title and identifier selectors while the retained source remains alive.
    unsafe {
        calendar.source().map_or((None, None), |source| {
            (
                Some(source.title().to_string()),
                Some(source.sourceIdentifier().to_string()),
            )
        })
    }
}

fn reminder_list_info(calendar: &EKCalendar, default: Option<&str>) -> ReminderListInfo {
    let id = calendar_id(calendar);
    let (source, source_id) = calendar_source(calendar);
    ReminderListInfo {
        default: default == Some(&id),
        id,
        name: calendar_title(calendar),
        source,
        source_id,
        kind: calendar_kind(calendar),
        writable: calendar_is_writable(calendar),
    }
}

fn calendar_info(calendar: &EKCalendar, default: Option<&str>) -> CalendarInfo {
    let id = calendar_id(calendar);
    let (source, source_id) = calendar_source(calendar);
    CalendarInfo {
        default: default == Some(&id),
        id,
        name: calendar_title(calendar),
        source,
        source_id,
        kind: calendar_kind(calendar),
        writable: calendar_is_writable(calendar),
    }
}

fn source_title(source: &EKSource) -> String {
    // SAFETY: A typed retained EKSource guarantees the title selector and NSString result.
    unsafe { source.title() }.to_string()
}

fn source_id(source: &EKSource) -> String {
    // SAFETY: A typed retained EKSource guarantees the identifier selector and NSString result.
    unsafe { source.sourceIdentifier() }.to_string()
}

fn source_type_can_create(kind: EKSourceType) -> bool {
    matches!(
        kind,
        EKSourceType::Local
            | EKSourceType::Exchange
            | EKSourceType::CalDAV
            | EKSourceType::MobileMe
    )
}

fn source_calendars(source: &EKSource, entity: EKEntityType) -> Vec<Retained<EKCalendar>> {
    // SAFETY: A typed retained EKSource guarantees this entity-specific calendar query, and the
    // returned set retains its EKCalendar elements while they are collected.
    unsafe { source.calendarsForEntityType(entity) }
        .iter()
        .collect()
}

fn source_is_eligible(entity: EKEntityType, source: &EKSource) -> bool {
    // SAFETY: A typed retained EKSource guarantees the source-type selector. Requiring an existing
    // mutable calendar from calendarsForEntityType deliberately rejects sources for which EventKit
    // has not demonstrated support for this entity, even if their broad source type looks writable.
    let can_create = source_type_can_create(unsafe { source.sourceType() });
    can_create
        && source_calendars(source, entity)
            .iter()
            .any(|calendar| !calendar_is_immutable(calendar))
}

fn sources(store: &EKEventStore) -> Vec<Retained<EKSource>> {
    // SAFETY: The retained store is valid and EventKit returns an array whose EKSource elements
    // remain retained while collected.
    unsafe { store.sources() }.iter().collect()
}

fn source_matches(source: &EKSource, selector: &str) -> bool {
    source_title(source) == selector || source_id(source) == selector
}

fn source_label(source: &EKSource) -> String {
    format!("{} [{}]", source_title(source), source_id(source))
}

fn default_source(store: &EKEventStore, entity: EKEntityType) -> Option<Retained<EKSource>> {
    let calendar = if entity == EKEntityType::Event {
        default_event_calendar(store)
    } else {
        default_reminder_calendar(store)
    };
    calendar.and_then(|calendar| {
        // SAFETY: A typed retained EKCalendar guarantees the source selector.
        unsafe { calendar.source() }
    })
}

fn resource_source(
    store: &EKEventStore,
    entity: EKEntityType,
    selector: Option<&str>,
) -> Result<Retained<EKSource>, BackendError> {
    let eligible = sources(store)
        .into_iter()
        .filter(|source| source_is_eligible(entity, source))
        .collect::<Vec<_>>();
    if let Some(selector) = selector {
        let mut matches = eligible
            .into_iter()
            .filter(|source| source_matches(source, selector))
            .collect::<Vec<_>>();
        return match matches.len() {
            0 => Err(BackendError::NotFound {
                kind: "writable EventKit source",
                name: selector.into(),
            }),
            1 => Ok(matches
                .pop()
                .expect("a one-element source vector must contain a source")),
            _ => Err(BackendError::Ambiguous {
                kind: "writable EventKit source",
                name: selector.into(),
                matches: matches.iter().map(|source| source_label(source)).collect(),
            }),
        };
    }
    if let Some(source) = default_source(store, entity)
        && source_is_eligible(entity, &source)
    {
        return Ok(source);
    }
    match eligible.as_slice() {
        [] => Err(BackendError::NotFound {
            kind: "writable EventKit source",
            name: "default".into(),
        }),
        [source] => Ok(source.clone()),
        _ => Err(BackendError::Ambiguous {
            kind: "writable EventKit source",
            name: "default".into(),
            matches: eligible.iter().map(|source| source_label(source)).collect(),
        }),
    }
}

fn create_calendar_resource(
    store: &EKEventStore,
    entity: EKEntityType,
    resource: &NewResource,
    kind: &'static str,
    app: &'static str,
) -> Result<Retained<EKCalendar>, BackendError> {
    let source = resource_source(store, entity, resource.owner.as_deref())?;
    ensure_unique_resource_name(&source, entity, kind, &resource.name, None)?;
    // SAFETY: The retained store and source are valid EventKit objects, the title NSString lives
    // through its copied setter, and the new calendar is saved before leaving the autorelease pool.
    let calendar = unsafe {
        let calendar = EKCalendar::calendarForEntityType_eventStore(entity, store);
        calendar.setTitle(&NSString::from_str(&resource.name));
        calendar.setSource(Some(&source));
        calendar
    };
    save_calendar(store, &calendar, app)?;
    Ok(calendar)
}

fn selected_calendar(
    store: &EKEventStore,
    entity: EKEntityType,
    selector: &ResourceSelector,
    kind: &'static str,
) -> Result<Retained<EKCalendar>, BackendError> {
    let available = calendars(store, entity);
    let (name, mut matches) = match selector {
        ResourceSelector::Id(id) => (
            id.as_str(),
            available
                .into_iter()
                .filter(|calendar| calendar_id(calendar) == *id)
                .collect::<Vec<_>>(),
        ),
        ResourceSelector::Name { name, owner } => {
            if let Some(owner) = owner
                && !sources(store)
                    .iter()
                    .any(|source| source_matches(source, owner))
            {
                return Err(BackendError::NotFound {
                    kind: "EventKit source",
                    name: owner.clone(),
                });
            }
            (
                name.as_str(),
                available
                    .into_iter()
                    .filter(|calendar| {
                        calendar_title(calendar) == *name
                            && owner.as_deref().is_none_or(|owner| {
                                // SAFETY: A typed retained EKCalendar guarantees the source selector.
                                unsafe { calendar.source() }
                                    .is_some_and(|source| source_matches(&source, owner))
                            })
                    })
                    .collect::<Vec<_>>(),
            )
        }
    };
    match matches.len() {
        0 => Err(BackendError::NotFound {
            kind,
            name: name.into(),
        }),
        1 => Ok(matches
            .pop()
            .expect("a one-element calendar vector must contain a calendar")),
        _ => Err(BackendError::Ambiguous {
            kind,
            name: name.into(),
            matches: matches
                .iter()
                .map(|calendar| {
                    let (source, _) = calendar_source(calendar);
                    format!(
                        "{} [{}]",
                        source.unwrap_or_else(|| "unknown source".into()),
                        calendar_id(calendar)
                    )
                })
                .collect(),
        }),
    }
}

fn rename_calendar_resource(
    store: &EKEventStore,
    entity: EKEntityType,
    selector: &ResourceSelector,
    name: &str,
    kind: &'static str,
    app: &'static str,
) -> Result<Retained<EKCalendar>, BackendError> {
    let calendar = selected_calendar(store, entity, selector, kind)?;
    // SAFETY: A typed retained EKCalendar guarantees the source selector.
    let source = unsafe { calendar.source() }
        .ok_or_else(|| failed(app, &format!("selected {kind} has no EventKit source")))?;
    let id = calendar_id(&calendar);
    ensure_unique_resource_name(&source, entity, kind, name, Some(&id))?;
    if calendar_title(&calendar) == name {
        return Ok(calendar);
    }
    if calendar_is_immutable(&calendar) {
        return Err(BackendError::Immutable {
            kind,
            name: calendar_title(&calendar),
        });
    }
    // SAFETY: EventKit copies the NSString during this synchronous title setter.
    unsafe { calendar.setTitle(&NSString::from_str(name)) };
    save_calendar(store, &calendar, app)?;
    Ok(calendar)
}

fn ensure_unique_resource_name(
    source: &EKSource,
    entity: EKEntityType,
    kind: &'static str,
    name: &str,
    current_id: Option<&str>,
) -> Result<(), BackendError> {
    let collision = source_calendars(source, entity).iter().any(|calendar| {
        resource_name_conflicts(
            &calendar_id(calendar),
            &calendar_title(calendar),
            current_id,
            name,
        )
    });
    if collision {
        Err(BackendError::AlreadyExists {
            kind,
            name: name.into(),
            owner: source_label(source),
        })
    } else {
        Ok(())
    }
}

fn resource_name_conflicts(
    candidate_id: &str,
    candidate_name: &str,
    current_id: Option<&str>,
    name: &str,
) -> bool {
    candidate_name == name && current_id != Some(candidate_id)
}

fn save_calendar(
    store: &EKEventStore,
    calendar: &EKCalendar,
    app: &'static str,
) -> Result<(), BackendError> {
    // SAFETY: The retained store owns the calendar and both remain valid through the synchronous save.
    unsafe { store.saveCalendar_commit_error(calendar, true) }
        .map_err(|error| failed(app, &error.localizedDescription().to_string()))
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
            matches: Vec::new(),
        }),
    }
}

fn stage(
    store: &EKEventStore,
    changes: &[ReminderChange],
) -> Result<Vec<Option<Retained<EKReminder>>>, BackendError> {
    let mut current = Vec::with_capacity(changes.len());
    let mut conflicts = Vec::new();
    for change in changes {
        match change {
            ReminderChange::Add { .. } => current.push(None),
            ReminderChange::Restore { before } => {
                if find_reminder(store, &before.id).is_none() {
                    current.push(None);
                } else {
                    conflicts.push(format!("\"{}\"", normalize_title(&before.title)));
                    current.push(None);
                }
            }
            ReminderChange::Update { before, .. } | ReminderChange::Delete { before } => {
                match find_reminder(store, &before.id) {
                    Some(reminder) if stored_matches(&to_stored(&reminder), before) => {
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
    let mut written = Vec::with_capacity(changes.len());
    for (change, reminder) in changes.iter().zip(current) {
        written.push(match (change, reminder) {
            (
                ReminderChange::Add {
                    list_id,
                    title,
                    notes,
                    due,
                    due_time,
                    due_time_zone,
                    priority,
                },
                _,
            ) => Some(stage_add(
                store,
                list_id,
                AddFields {
                    title,
                    notes: notes.as_deref(),
                    due: *due,
                    due_time: *due_time,
                    due_time_zone: due_time_zone.as_deref(),
                    priority: *priority,
                },
            )?),
            (ReminderChange::Restore { before }, _) => Some(stage_restore(store, before)?),
            (ReminderChange::Update { before, edit }, Some(reminder)) => {
                update(store, &reminder, before, edit)?;
                save(store, &reminder, &before.title)?;
                Some(reminder)
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
                None
            }
            _ => unreachable!("conflicts were checked above"),
        });
    }
    Ok(written)
}

fn stored_matches(current: &StoredReminder, before: &StoredReminder) -> bool {
    let mut current = current.clone();
    if before.absolute_alarms.is_none() {
        current.absolute_alarms = None;
    }
    current == *before
}

#[derive(Clone, Copy)]
struct AddFields<'a> {
    title: &'a str,
    notes: Option<&'a str>,
    due: Option<Date>,
    due_time: Option<Time>,
    due_time_zone: Option<&'a str>,
    priority: Option<Priority>,
}

fn stage_add(
    store: &EKEventStore,
    list_id: &str,
    fields: AddFields<'_>,
) -> Result<Retained<EKReminder>, BackendError> {
    let list = writable_calendar(store, list_id)?;
    // SAFETY: The retained EventKit store and calendar are valid, temporary NSString values live
    // through each setter call, and the reminder is staged before any object can be released.
    unsafe {
        let reminder = EKReminder::reminderWithEventStore(store);
        reminder.setCalendar(Some(&list));
        reminder.setTitle(Some(&NSString::from_str(fields.title)));
        reminder.setNotes(fields.notes.map(NSString::from_str).as_deref());
        reminder.setPriority(priority_to_eventkit(fields.priority));
        set_due(&reminder, fields.due, fields.due_time, fields.due_time_zone)?;
        save(store, &reminder, fields.title)?;
        Ok(reminder)
    }
}

fn stage_restore(
    store: &EKEventStore,
    before: &StoredReminder,
) -> Result<Retained<EKReminder>, BackendError> {
    let list = writable_calendar(store, &before.list_id)?;
    // SAFETY: The retained EventKit store and calendar are valid, temporary Foundation values live
    // through their setter calls, and the new reminder is staged before it can be released.
    unsafe {
        let reminder = EKReminder::reminderWithEventStore(store);
        reminder.setCalendar(Some(&list));
        reminder.setTitle(Some(&NSString::from_str(&before.title)));
        reminder.setNotes(before.notes.as_deref().map(NSString::from_str).as_deref());
        reminder.setPriority(priority_to_eventkit(before.priority));
        if let Some(parts) = &before.due_components {
            let restored = restore_due_components(parts)?;
            reminder.setDueDateComponents(Some(&restored));
        } else {
            set_due(&reminder, before.due, None, None)?;
        }
        let url = before
            .url
            .as_deref()
            .and_then(|url| NSURL::URLWithString(&NSString::from_str(url)));
        reminder.setURL(url.as_deref());
        reminder.setCompleted(before.completed);
        save(store, &reminder, &before.title)?;
        Ok(reminder)
    }
}

fn update(
    store: &EKEventStore,
    reminder: &EKReminder,
    before: &StoredReminder,
    edit: &ReminderEdit,
) -> Result<(), BackendError> {
    // SAFETY: The typed reminder and store are retained for every getter and setter call, an
    // existing due-components object remains alive while its scalar fields are updated, temporary
    // NSString values outlive their calls, and any destination calendar is retained before use.
    unsafe {
        if let Some(title) = &edit.title {
            reminder.setTitle(Some(&NSString::from_str(title)));
        }
        match &edit.notes {
            FieldEdit::Set(notes) => reminder.setNotes(Some(&NSString::from_str(notes))),
            FieldEdit::Remove => reminder.setNotes(None),
            FieldEdit::Unchanged => {}
        }
        if let Some(parts) = &edit.due_components {
            let component_objects_unchanged =
                before.due_components.as_ref().is_some_and(|before| {
                    before.calendar == parts.calendar && before.time_zone == parts.time_zone
                });
            if component_objects_unchanged && let Some(existing) = reminder.dueDateComponents() {
                set_due_component_values(&existing, parts);
                reminder.setDueDateComponents(Some(&existing));
            } else {
                let restored = restore_due_components(parts)?;
                reminder.setDueDateComponents(Some(&restored));
            }
        } else {
            match edit.due {
                FieldEdit::Set(due) => set_due(reminder, Some(due), None, None)?,
                FieldEdit::Remove => set_due(reminder, None, None, None)?,
                FieldEdit::Unchanged => {}
            }
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
        if let Some(completed) = edit.completed {
            reminder.setCompleted(completed);
        }
    }
    Ok(())
}

fn restore_due_components(
    value: &ReminderDueComponents,
) -> Result<Retained<NSDateComponents>, BackendError> {
    let parts = NSDateComponents::new();
    let calendar = match value.calendar.as_deref() {
        Some(identifier) => NSCalendar::calendarWithIdentifier(&NSString::from_str(identifier))
            .ok_or_else(|| {
                failed(
                    REMINDERS,
                    &format!("saved calendar {identifier:?} isn't available"),
                )
            })?,
        None => gregorian_calendar()
            .ok_or_else(|| failed(REMINDERS, "the Gregorian calendar isn't available"))?,
    };
    parts.setCalendar(Some(&calendar));
    let time_zone = value
        .time_zone
        .as_deref()
        .map(|name| {
            NSTimeZone::timeZoneWithName(&NSString::from_str(name)).ok_or_else(|| {
                failed(
                    REMINDERS,
                    &format!("saved time zone {name:?} isn't available"),
                )
            })
        })
        .transpose()?;
    parts.setTimeZone(time_zone.as_deref());
    set_due_component_values(&parts, value);
    Ok(parts)
}

fn set_due_component_values(parts: &NSDateComponents, value: &ReminderDueComponents) {
    if let Some(value) = value.era {
        parts.setEra(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setEra(NSDateComponentUndefined);
    }
    if let Some(value) = value.year {
        parts.setYear(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setYear(NSDateComponentUndefined);
    }
    if let Some(value) = value.month {
        parts.setMonth(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setMonth(NSDateComponentUndefined);
    }
    if let Some(value) = value.day {
        parts.setDay(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setDay(NSDateComponentUndefined);
    }
    if let Some(value) = value.hour {
        parts.setHour(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setHour(NSDateComponentUndefined);
    }
    if let Some(value) = value.minute {
        parts.setMinute(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setMinute(NSDateComponentUndefined);
    }
    if let Some(value) = value.second {
        parts.setSecond(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setSecond(NSDateComponentUndefined);
    }
    if let Some(value) = value.nanosecond {
        parts.setNanosecond(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setNanosecond(NSDateComponentUndefined);
    }
    if let Some(value) = value.weekday {
        parts.setWeekday(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setWeekday(NSDateComponentUndefined);
    }
    if let Some(value) = value.weekday_ordinal {
        parts.setWeekdayOrdinal(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setWeekdayOrdinal(NSDateComponentUndefined);
    }
    if let Some(value) = value.quarter {
        parts.setQuarter(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setQuarter(NSDateComponentUndefined);
    }
    if let Some(value) = value.week_of_month {
        parts.setWeekOfMonth(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setWeekOfMonth(NSDateComponentUndefined);
    }
    if let Some(value) = value.week_of_year {
        parts.setWeekOfYear(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setWeekOfYear(NSDateComponentUndefined);
    }
    if let Some(value) = value.year_for_week_of_year {
        parts.setYearForWeekOfYear(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setYearForWeekOfYear(NSDateComponentUndefined);
    }
    if let Some(value) = value.day_of_year {
        parts.setDayOfYear(value.try_into().unwrap_or(NSDateComponentUndefined));
    } else {
        parts.setDayOfYear(NSDateComponentUndefined);
    }
    parts.setLeapMonth(value.is_leap_month);
}

fn set_due(
    reminder: &EKReminder,
    due: Option<Date>,
    time: Option<Time>,
    time_zone: Option<&str>,
) -> Result<(), BackendError> {
    // SAFETY: The typed reminder is retained for all accesses, and the newly built or retained
    // NSDateComponents object remains alive through assignment back to the reminder.
    unsafe {
        let Some(date) = due else {
            reminder.setDueDateComponents(None);
            return Ok(());
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
        if let Some(time) = time {
            parts.setHour(time.hour().into());
            parts.setMinute(time.minute().into());
            parts.setSecond(0);
            parts.setNanosecond(0);
            let zone = time_zone
                .map(|name| {
                    NSTimeZone::timeZoneWithName(&NSString::from_str(name)).ok_or_else(|| {
                        failed(REMINDERS, &format!("time zone {name:?} isn't available"))
                    })
                })
                .transpose()?;
            parts.setTimeZone(zone.as_deref());
        }
        reminder.setDueDateComponents(Some(&parts));
    }
    Ok(())
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
    // SAFETY: Reminder selectors are invoked on a typed retained EKReminder. The alarm array's
    // typed EKAlarm elements stay retained while absoluteDate is called, and all Objective-C values
    // are copied into owned Rust values before this function returns.
    unsafe {
        let due = reminder.dueDateComponents();
        let absolute_alarms = reminder
            .alarms()
            .map(|alarms| {
                alarms
                    .iter()
                    .filter_map(|alarm| alarm.absoluteDate())
                    .filter_map(|date| timestamp_from_epoch(date.timeIntervalSince1970()))
                    .collect()
            })
            .unwrap_or_default();
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
            completed: reminder.isCompleted(),
            modified: reminder
                .lastModifiedDate()
                .and_then(|d| timestamp_from_epoch(d.timeIntervalSince1970())),
            alarms_present: reminder.hasAlarms(),
            absolute_alarms: Some(absolute_alarms),
            recurrence_present: reminder.hasRecurrenceRules(),
            backup_partial: reminder.hasAlarms()
                || reminder.hasRecurrenceRules()
                || (url_object.is_some() && url.is_none()),
            url,
        }
    }
}

fn stored_due_time(parts: &ReminderDueComponents) -> Option<Time> {
    time_from_parts(parts.hour?.try_into().ok()?, parts.minute?.try_into().ok()?)
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

pub(super) fn reminders_authorization_status() -> PermissionStatus {
    permission_status(authorization_status(EKEntityType::Reminder))
}

pub(super) fn calendars_authorization_status() -> PermissionStatus {
    permission_status(authorization_status(EKEntityType::Event))
}

pub(super) fn start_new_session() {
    // SAFETY: setsid takes no pointers and only changes this refresh process's session membership.
    unsafe {
        let _ = libc::setsid();
    }
}

pub(super) fn completion_reminder_list_names() -> Option<(Vec<String>, Vec<String>)> {
    if authorization_status(EKEntityType::Reminder) != EKAuthorizationStatus::FullAccess {
        return None;
    }
    Some(autoreleasepool(|_| {
        let store = new_event_store();
        reminder_list_names(&store)
    }))
}

pub(super) fn completion_calendar_names() -> Option<Vec<String>> {
    if authorization_status(EKEntityType::Event) != EKAuthorizationStatus::FullAccess {
        return None;
    }
    Some(autoreleasepool(|_| {
        let store = new_event_store();
        calendar_names(&store, EKEntityType::Event)
    }))
}

fn permission_status(status: EKAuthorizationStatus) -> PermissionStatus {
    match status {
        EKAuthorizationStatus::FullAccess => PermissionStatus::FullAccess,
        EKAuthorizationStatus::NotDetermined => PermissionStatus::NotDetermined,
        EKAuthorizationStatus::Restricted => PermissionStatus::Restricted,
        EKAuthorizationStatus::Denied => PermissionStatus::Denied,
        EKAuthorizationStatus::WriteOnly => PermissionStatus::WriteOnly,
        EKAuthorizationStatus(value) => PermissionStatus::Unknown(format!("status {value}")),
    }
}

pub(super) fn notes_automation_status() -> AutomationStatus {
    let mut target = AppleEventDescriptor {
        descriptor_type: 0,
        data_handle: std::ptr::null_mut(),
    };
    let size = isize::try_from(NOTES_BUNDLE_ID.len())
        .expect("the fixed Notes bundle identifier length fits in isize");
    // SAFETY: The bundle identifier bytes are valid for `size`, `target` points to initialized
    // descriptor storage, and the Apple Event Manager copies the bytes into owned storage.
    let created = unsafe {
        ae_create_desc(
            TYPE_APPLICATION_BUNDLE_ID,
            NOTES_BUNDLE_ID.as_ptr().cast(),
            size,
            &raw mut target,
        )
    };
    if created != 0 {
        return AutomationStatus::Unknown(format!("AECreateDesc returned {created}"));
    }
    // SAFETY: `target` was successfully initialized by `AECreateDesc`, wildcard event codes are
    // explicitly supported, and a false final argument guarantees that macOS won't prompt.
    let status =
        unsafe { ae_determine_permission(&raw const target, TYPE_WILDCARD, TYPE_WILDCARD, 0) };
    // SAFETY: `target` was successfully initialized by `AECreateDesc` and is disposed exactly once.
    let disposed = unsafe { ae_dispose_desc(&raw mut target) };
    if disposed != 0 {
        return AutomationStatus::Unknown(format!("AEDisposeDesc returned {disposed}"));
    }
    match status {
        NO_ERROR => AutomationStatus::Allowed,
        EVENT_NOT_PERMITTED => AutomationStatus::Denied,
        EVENT_WOULD_REQUIRE_CONSENT => AutomationStatus::NotDetermined,
        PROCESS_NOT_FOUND => AutomationStatus::TargetNotRunning,
        value => AutomationStatus::Unknown(format!("Apple Events status {value}")),
    }
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
            calendar: event
                .calendar()
                .map(|calendar| calendar_title(&calendar))
                .unwrap_or_default(),
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
        let due = reminder.dueDateComponents();
        Reminder {
            id: reminder.calendarItemIdentifier().to_string(),
            title: reminder.title().to_string(),
            due: due
                .as_ref()
                .and_then(|parts| date_from_parts(parts.year(), parts.month(), parts.day())),
            due_time: due
                .as_ref()
                .and_then(|parts| time_from_parts(parts.hour(), parts.minute())),
            due_time_zone: due
                .as_ref()
                .and_then(|parts| parts.timeZone())
                .map(|zone| zone.name().to_string()),
            due_at: due.as_ref().and_then(|_| due_timestamp(reminder)),
            priority: priority_from_eventkit(reminder.priority()),
        }
    }
}

fn due_timestamp(reminder: &EKReminder) -> Option<Timestamp> {
    // SAFETY: The typed retained EKReminder and its returned NSDateComponents, NSCalendar, and
    // NSDate stay alive through the synchronous conversion; the timestamp is copied into Rust.
    unsafe {
        let parts = reminder.dueDateComponents()?;
        time_from_parts(parts.hour(), parts.minute())?;
        let calendar = parts.calendar().or_else(gregorian_calendar)?;
        let date = calendar.dateFromComponents(&parts)?;
        timestamp_from_epoch(date.timeIntervalSince1970())
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

fn time_from_parts(hour: NSInteger, minute: NSInteger) -> Option<Time> {
    if [hour, minute].contains(&NSDateComponentUndefined) {
        return None;
    }
    Time::new(hour.try_into().ok()?, minute.try_into().ok()?, 0, 0).ok()
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
    use jiff::tz::offset;
    use proptest::prelude::*;

    use super::*;
    use crate::app::markdown;

    #[test]
    fn only_mutable_account_source_types_are_creation_candidates() {
        for source in [
            EKSourceType::Local,
            EKSourceType::Exchange,
            EKSourceType::CalDAV,
            EKSourceType::MobileMe,
        ] {
            assert!(source_type_can_create(source));
        }
        for source in [EKSourceType::Subscribed, EKSourceType::Birthdays] {
            assert!(!source_type_can_create(source));
        }
    }

    #[test]
    fn resource_name_collision_excludes_only_the_selected_resource() {
        assert!(resource_name_conflicts(
            "other",
            "Work",
            Some("current"),
            "Work"
        ));
        assert!(!resource_name_conflicts(
            "current",
            "Work",
            Some("current"),
            "Work"
        ));
        assert!(!resource_name_conflicts(
            "other",
            "Personal",
            Some("current"),
            "Work"
        ));
    }

    struct LiveReminderCleanup {
        id: Option<String>,
        list_id: String,
        list_name: String,
        title: String,
    }

    impl Drop for LiveReminderCleanup {
        fn drop(&mut self) {
            let id = self.id.clone().or_else(|| {
                EventKitReminders::uncached()
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
    fn reminders_sort_by_displayed_due_fields_title_and_id() {
        let mut reminders = [
            Reminder {
                id: "undated".into(),
                title: "First alphabetically".into(),
                due: None,
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: None,
            },
            Reminder {
                id: "date-only".into(),
                title: "Z date only".into(),
                due: Some(date(2026, 11, 3)),
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: None,
            },
            sort_reminder("b", "Same", 3, Some(9), -5),
            sort_reminder("a", "Same", 3, Some(9), 9),
            sort_reminder("later-time", "A", 3, Some(10), 0),
            sort_reminder("later-date", "A", 4, Some(1), 9),
        ];
        reminders.sort_by(compare_reminders);
        assert_eq!(
            reminders
                .iter()
                .map(|reminder| reminder.id.as_str())
                .collect::<Vec<_>>(),
            ["date-only", "a", "b", "later-time", "later-date", "undated"]
        );
    }

    fn sort_reminder(
        id: &str,
        title: &str,
        day: i8,
        hour: Option<i8>,
        offset_hours: i8,
    ) -> Reminder {
        let due = date(2026, 11, day);
        let due_time = hour.map(|hour| jiff::civil::Time::new(hour, 0, 0, 0).unwrap());
        let zone = TimeZone::fixed(offset(offset_hours));
        let due_at = due_time.map(|time| due.to_datetime(time).to_zoned(zone).unwrap().timestamp());
        Reminder {
            id: id.into(),
            title: title.into(),
            due: Some(due),
            due_time,
            due_time_zone: due_time.map(|_| format!("zone-{offset_hours}")),
            due_at,
            priority: None,
        }
    }

    proptest! {
        #[test]
        fn sorting_randomized_mixed_zones_never_panics_and_is_deterministic(
            rows in proptest::collection::vec(
                (1_i8..=28, proptest::option::of(0_i8..24), 0_usize..4, "[A-Z]{0,8}"),
                18..80,
            )
        ) {
            let offsets = [-10, -5, 0, 9];
            let mut reminders = rows
                .into_iter()
                .enumerate()
                .map(|(index, (day, hour, zone, title))| {
                    sort_reminder(
                        &format!("random-{index:03}"),
                        &title,
                        day,
                        hour,
                        offsets[zone],
                    )
                })
                .collect::<Vec<_>>();
            reminders.extend([
                sort_reminder("tokyo", "A", 4, Some(1), 9),
                sort_reminder("new-york", "C", 3, Some(20), -5),
                sort_reminder("date-only", "B", 4, None, 0),
            ]);
            let mut first = reminders.clone();
            let mut second = reminders;
            first.sort_by(compare_reminders);
            second.sort_by(compare_reminders);
            prop_assert_eq!(
                first.iter().map(|reminder| &reminder.id).collect::<Vec<_>>(),
                second.iter().map(|reminder| &reminder.id).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn unrecorded_alarm_data_is_ignored_during_conflict_checks() {
        let mut current = StoredReminder {
            id: "r1".into(),
            list_id: "l1".into(),
            title: "Wake up".into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            completed: false,
            modified: None,
            alarms_present: true,
            absolute_alarms: Some(vec!["2026-11-03T14:00:00Z".parse().unwrap()]),
            recurrence_present: false,
            url: None,
            backup_partial: true,
        };
        let mut old_record = current.clone();
        old_record.absolute_alarms = None;
        assert!(stored_matches(&current, &old_record));
        old_record.absolute_alarms = Some(Vec::new());
        assert!(!stored_matches(&current, &old_record));
        current.absolute_alarms = Some(Vec::new());
        assert!(stored_matches(&current, &old_record));
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
        let backend = EventKitCalendar::uncached();
        backend
            .events(EventQuery {
                range: start..end,
                calendars: Vec::new(),
            })
            .unwrap();
        backend.calendars().unwrap();
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
    fn restored_due_components_keep_the_captured_time_and_zone() {
        let captured = ReminderDueComponents {
            calendar: Some("gregorian".into()),
            time_zone: Some("America/Chicago".into()),
            year: Some(2026),
            month: Some(10),
            day: Some(27),
            hour: Some(14),
            minute: Some(30),
            second: Some(15),
            ..ReminderDueComponents::default()
        };
        let restored = restore_due_components(&captured).unwrap();
        assert_eq!(restored.year(), 2026);
        assert_eq!(restored.month(), 10);
        assert_eq!(restored.day(), 27);
        assert_eq!(restored.hour(), 14);
        assert_eq!(restored.minute(), 30);
        assert_eq!(restored.second(), 15);
        assert_eq!(
            restored.timeZone().unwrap().name().to_string(),
            "America/Chicago"
        );
    }

    #[test]
    fn restored_due_components_keep_the_captured_calendar() {
        let restored = restore_due_components(&ReminderDueComponents {
            calendar: Some("buddhist".into()),
            year: Some(2569),
            month: Some(10),
            day: Some(27),
            ..ReminderDueComponents::default()
        })
        .unwrap();
        assert_eq!(
            restored
                .calendar()
                .unwrap()
                .calendarIdentifier()
                .to_string(),
            "buddhist"
        );
    }

    #[test]
    fn restoring_an_unknown_saved_time_zone_fails() {
        let error = restore_due_components(&ReminderDueComponents {
            time_zone: Some("Not/A-Time-Zone".into()),
            ..ReminderDueComponents::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("Not/A-Time-Zone"));
    }

    #[test]
    #[ignore = "writes to your real Reminders; set NIB_LIVE_WRITE=1 and run with --ignored"]
    fn adds_and_removes_a_real_reminder() {
        if std::env::var_os("NIB_LIVE_WRITE").is_none() {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Reminders");
            return;
        }
        let added = EventKitReminders::uncached()
            .add(NewReminder {
                title: "nib live test (safe to delete)".into(),
                list: None,
                due: Some(date(2030, 1, 15)),
                due_time: None,
                due_time_zone: None,
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
    fn edits_and_undoes_a_real_reminder() {
        if std::env::var_os("NIB_LIVE_WRITE").is_none() {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Reminders");
            return;
        }
        let store = authorized_store(EKEntityType::Reminder, REMINDERS).unwrap();
        let list =
            default_reminder_calendar(&store).expect("there is no default list for new reminders");
        let list_id = calendar_id(&list);
        let list_name = calendar_title(&list);
        let title = format!("nib undo live test {}", std::process::id());
        let mut cleanup = LiveReminderCleanup {
            id: None,
            list_id: list_id.clone(),
            list_name,
            title: title.clone(),
        };
        let added = EventKitReminders::uncached()
            .apply(&[ReminderChange::Add {
                list_id,
                title: title.clone(),
                notes: None,
                due: None,
                due_time: None,
                due_time_zone: None,
                priority: None,
            }])
            .unwrap()
            .written
            .into_iter()
            .next()
            .flatten()
            .unwrap();
        cleanup.id = Some(added.id.clone());
        let edited_title = format!("{title} edited");
        let edited = EventKitReminders::uncached()
            .apply(&[ReminderChange::Update {
                before: added.clone(),
                edit: ReminderEdit {
                    title: Some(edited_title),
                    ..ReminderEdit::default()
                },
            }])
            .unwrap()
            .written
            .into_iter()
            .next()
            .flatten()
            .unwrap();
        let restored = EventKitReminders::uncached()
            .apply(&[ReminderChange::Update {
                before: edited,
                edit: ReminderEdit {
                    title: Some(title.clone()),
                    ..ReminderEdit::default()
                },
            }])
            .unwrap()
            .written
            .into_iter()
            .next()
            .flatten()
            .unwrap();
        assert_eq!(restored.title, title);
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

        EventKitReminders::uncached()
            .apply(&[ReminderChange::Add {
                list_id: list_id.clone(),
                title: title.clone(),
                notes: Some(notes.into()),
                due: None,
                due_time: None,
                due_time_zone: None,
                priority: None,
            }])
            .unwrap();

        let snapshot = EventKitReminders::uncached()
            .snapshot(Some(&list_name))
            .unwrap();
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
                    && edit.completed == Some(true)
        ));
        EventKitReminders::uncached().apply(&plan.changes).unwrap();

        let after = EventKitReminders::uncached()
            .snapshot(Some(&list_name))
            .unwrap();
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
        let backend = EventKitReminders::uncached();
        backend.list().unwrap();
        backend.lists().unwrap();
    }
}

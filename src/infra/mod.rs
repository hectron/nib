pub(crate) mod completions;
pub(crate) mod config;
pub(crate) mod demo;
pub(crate) mod doctor;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod eventkit;
pub(crate) mod host;
mod notes;

#[cfg(any(not(target_os = "macos"), test))]
use crate::domain::{
    ApplyResult, BackendError, CalendarInfo, Event, EventQuery, NewNote, NewReminder, Note,
    NoteFolder, NoteQuery, Reminder, ReminderChange, ReminderSnapshot,
};
use crate::domain::{Backends, Calendar, Notes, Reminders};

pub(crate) fn system_backends(cache: completions::CacheFile) -> Backends {
    let notes_cache = cache.clone();
    let reminders_cache = cache.clone();
    Backends::new(
        move || system_notes(notes_cache),
        move || system_reminders(reminders_cache),
        move || system_calendar(cache),
    )
}

#[cfg(target_os = "macos")]
fn system_notes(cache: completions::CacheFile) -> Box<dyn Notes> {
    Box::new(notes::AppleNotes::new(cache))
}

#[cfg(not(target_os = "macos"))]
fn system_notes(_: completions::CacheFile) -> Box<dyn Notes> {
    Box::new(Unimplemented("Apple Notes"))
}

#[cfg(target_os = "macos")]
fn system_calendar(cache: completions::CacheFile) -> Box<dyn Calendar> {
    Box::new(eventkit::EventKitCalendar::new(cache))
}

#[cfg(not(target_os = "macos"))]
fn system_calendar(_: completions::CacheFile) -> Box<dyn Calendar> {
    Box::new(Unimplemented("Calendar"))
}

#[cfg(target_os = "macos")]
fn system_reminders(cache: completions::CacheFile) -> Box<dyn Reminders> {
    Box::new(eventkit::EventKitReminders::new(cache))
}

#[cfg(not(target_os = "macos"))]
fn system_reminders(_: completions::CacheFile) -> Box<dyn Reminders> {
    Box::new(Unimplemented("Reminders"))
}

#[cfg(any(not(target_os = "macos"), test))]
struct Unimplemented(&'static str);

#[cfg(any(not(target_os = "macos"), test))]
impl Unimplemented {
    fn fail<T>(&self) -> Result<T, BackendError> {
        Err(BackendError::NotImplemented { app: self.0 })
    }
}

#[cfg(any(not(target_os = "macos"), test))]
impl Notes for Unimplemented {
    fn create(&self, _: NewNote) -> Result<Note, BackendError> {
        self.fail()
    }

    fn list(&self, _: NoteQuery) -> Result<Vec<Note>, BackendError> {
        self.fail()
    }

    fn folders(&self) -> Result<Vec<NoteFolder>, BackendError> {
        self.fail()
    }
}

#[cfg(any(not(target_os = "macos"), test))]
impl Reminders for Unimplemented {
    fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
        self.fail()
    }

    fn list(&self) -> Result<Vec<Reminder>, BackendError> {
        self.fail()
    }

    fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
        self.fail()
    }

    fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
        self.fail()
    }

    fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
        self.fail()
    }
}

#[cfg(any(not(target_os = "macos"), test))]
impl Calendar for Unimplemented {
    fn events(&self, _: EventQuery) -> Result<Vec<Event>, BackendError> {
        self.fail()
    }

    fn calendars(&self) -> Result<Vec<CalendarInfo>, BackendError> {
        self.fail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_notes_fail_without_external_io() {
        let backend = Unimplemented("Apple Notes");
        let error = Notes::list(&backend, NoteQuery { folder: None }).unwrap_err();
        assert!(matches!(
            error,
            BackendError::NotImplemented { app: "Apple Notes" }
        ));
    }
}

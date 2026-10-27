pub(crate) mod config;
pub(crate) mod demo;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod eventkit;
pub(crate) mod host;

use crate::domain::{
    BackendError, Backends, Calendar, Event, EventQuery, NewNote, NewReminder, Note, NoteQuery,
    Notes, Reminder, ReminderChange, ReminderSnapshot, Reminders,
};

pub(crate) fn system_backends() -> Backends {
    Backends::new(
        || Box::new(Unimplemented("Apple Notes")),
        system_reminders,
        system_calendar,
    )
}

#[cfg(target_os = "macos")]
fn system_calendar() -> Box<dyn Calendar> {
    Box::new(eventkit::EventKitCalendar)
}

#[cfg(not(target_os = "macos"))]
fn system_calendar() -> Box<dyn Calendar> {
    Box::new(Unimplemented("Calendar"))
}

#[cfg(target_os = "macos")]
fn system_reminders() -> Box<dyn Reminders> {
    Box::new(eventkit::EventKitReminders)
}

#[cfg(not(target_os = "macos"))]
fn system_reminders() -> Box<dyn Reminders> {
    Box::new(Unimplemented("Reminders"))
}

struct Unimplemented(&'static str);

impl Unimplemented {
    fn fail<T>(&self) -> Result<T, BackendError> {
        Err(BackendError::NotImplemented { app: self.0 })
    }
}

impl Notes for Unimplemented {
    fn create(&self, _: NewNote) -> Result<Note, BackendError> {
        self.fail()
    }

    fn list(&self, _: NoteQuery) -> Result<Vec<Note>, BackendError> {
        self.fail()
    }
}

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

    fn apply(&self, _: &[ReminderChange]) -> Result<(), BackendError> {
        self.fail()
    }
}

impl Calendar for Unimplemented {
    fn events(&self, _: EventQuery) -> Result<Vec<Event>, BackendError> {
        self.fail()
    }
}

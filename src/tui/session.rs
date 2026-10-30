use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::{Timestamp, Zoned};
use miette::Result;

use super::state::{Data, Effect, Item, ListChoice, Response};
use crate::app::commands::reminder_from_snapshot;
use crate::app::quick::{self, DeleteError};
use crate::cli::output::{self, Style, clean};
use crate::domain::{Backends, Config, Reminder, ReminderSnapshot, TaskChangeRequest};
use crate::infra::host::{EditFile, Host, JournalFile, JournalLock};

pub(crate) struct Session<'a> {
    pub now: &'a Zoned,
    pub config: &'a Config,
    pub backends: &'a Backends,
    pub host: &'a mut dyn Host,
}

impl Session<'_> {
    pub(crate) fn load(&self, includes_completed: bool) -> Result<Data> {
        let snapshot = if includes_completed {
            self.backends.reminders.snapshot_including_completed()?
        } else {
            self.backends.reminders.snapshot(None)?
        };
        Ok(data(snapshot, includes_completed, self.now))
    }

    pub(crate) fn perform(&mut self, effect: Effect, includes_completed: bool) -> Response {
        let result = match effect {
            Effect::LoadCompleted => self.load(true).map(Response::Loaded),
            Effect::Add(text) => self.add(&text),
            Effect::Change { change, ids } => {
                let (now, backends) = (self.now, self.backends);
                let request = TaskChangeRequest {
                    change,
                    selectors: ids,
                    list: None,
                };
                self.quietly(|host| {
                    let changed = quick::run(&request, now, backends, host)?;
                    let message = output::changed_reminders(&changed, false, Style::default())?;
                    Ok(message.lines().collect::<Vec<_>>().join("; "))
                })
            }
            Effect::Delete {
                ids,
                partial_confirmed,
            } => {
                let (now, backends) = (self.now, self.backends);
                self.quietly(|host| {
                    let deleted = quick::delete(&ids, partial_confirmed, now, backends, host)?;
                    Ok(match deleted.as_slice() {
                        [] => "No changes.".to_owned(),
                        [only] => format!("Deleted \"{}\"", clean(&only.title)),
                        many => format!("Deleted {} reminders", many.len()),
                    })
                })
            }
        };
        match result {
            Ok(Response::Changed { message, error, .. }) => {
                self.reloaded(message, error, includes_completed)
            }
            Ok(other) => other,
            Err(error) => match error.downcast_ref::<DeleteError>() {
                Some(DeleteError::PartialBackup { ids, .. }) => {
                    self.refused(clean(&error.to_string()), ids.clone(), includes_completed)
                }
                _ => Response::Failed(clean(&error.to_string())),
            },
        }
    }

    fn refused(&self, message: String, partial: Vec<String>, includes_completed: bool) -> Response {
        let (message, data) = self.reload(message, includes_completed);
        Response::DeleteRefused {
            message,
            partial,
            data,
        }
    }

    fn reloaded(&self, message: String, error: bool, includes_completed: bool) -> Response {
        let (message, data) = self.reload(message, includes_completed);
        Response::Changed {
            message,
            error: error || data.is_none(),
            data,
        }
    }

    fn reload(&self, message: String, includes_completed: bool) -> (String, Option<Data>) {
        match self.load(includes_completed) {
            Ok(data) => (message, Some(data)),
            Err(reload) => (
                format!(
                    "{message} Couldn't reload reminders, so the list may be out of date: {}",
                    clean(&reload.to_string())
                ),
                None,
            ),
        }
    }

    fn quietly(&mut self, run: impl FnOnce(&mut dyn Host) -> Result<String>) -> Result<Response> {
        let mut host = Quiet {
            inner: &mut *self.host,
            said: Vec::new(),
            unrecorded: false,
        };
        let result = run(&mut host);
        let said = host.said.iter().map(|line| clean(line));
        match result {
            Ok(summary) => Ok(Response::Changed {
                message: std::iter::once(summary)
                    .chain(said)
                    .collect::<Vec<_>>()
                    .join(" "),
                error: false,
                data: None,
            }),
            Err(error) if host.said.is_empty() => Err(error),
            Err(error) => {
                let message = said
                    .chain(std::iter::once(error.to_string()))
                    .collect::<Vec<_>>()
                    .join(" ");
                if host.unrecorded {
                    Ok(Response::Changed {
                        message,
                        error: true,
                        data: None,
                    })
                } else {
                    Err(miette::miette!("{message}"))
                }
            }
        }
    }

    fn add(&mut self, text: &str) -> Result<Response> {
        let new = crate::cli::quick_add(text, self.now, self.config)
            .map_err(|error| miette::miette!("{error}"))?;
        let list = new.list.clone();
        let added = self.backends.reminders.add(new)?;
        Ok(Response::Added(added_item(added, list)))
    }
}

fn data(snapshot: ReminderSnapshot, includes_completed: bool, now: &Zoned) -> Data {
    let ReminderSnapshot { lists, reminders } = snapshot;
    let items = reminders
        .into_iter()
        .map(|stored| {
            let list = lists
                .iter()
                .find(|list| list.id == stored.list_id)
                .map_or_else(|| stored.list_id.clone(), |list| list.name.clone());
            let notes = stored.notes.clone();
            let url = stored.url.clone();
            let completed = stored.completed;
            let backup_partial = stored.backup_partial;
            let reminder = reminder_from_snapshot(stored, now.time_zone());
            Item {
                id: reminder.id,
                title: reminder.title,
                list: Some(list),
                notes,
                url,
                due: reminder.due,
                due_time: reminder.due_time,
                due_time_zone: reminder.due_time_zone,
                due_at: reminder.due_at,
                priority: reminder.priority,
                completed,
                backup_partial,
            }
        })
        .collect();
    Data {
        items,
        lists: lists
            .into_iter()
            .filter(|list| list.writable)
            .map(|list| ListChoice {
                id: list.id,
                name: list.name,
            })
            .collect(),
        includes_completed,
    }
}

fn added_item(reminder: Reminder, list: Option<String>) -> Item {
    Item {
        id: reminder.id,
        title: reminder.title,
        list,
        notes: None,
        url: None,
        due: reminder.due,
        due_time: reminder.due_time,
        due_time_zone: reminder.due_time_zone,
        due_at: reminder.due_at,
        priority: reminder.priority,
        completed: false,
        backup_partial: false,
    }
}

struct Quiet<'a> {
    inner: &'a mut dyn Host,
    said: Vec<String>,
    unrecorded: bool,
}

impl Host for Quiet<'_> {
    fn create_edit_file(&mut self, contents: &str) -> io::Result<EditFile> {
        self.inner.create_edit_file(contents)
    }

    fn edit(&mut self, path: &Path) -> io::Result<Duration> {
        self.inner.edit(path)
    }

    fn ask(&mut self, _prompt: &str) -> io::Result<String> {
        Ok("n".into())
    }

    fn say(&mut self, message: &str) {
        self.said.push(message.to_owned());
    }

    fn say_unrecorded(&mut self, message: &str) {
        self.unrecorded = true;
        self.said.push(message.to_owned());
    }

    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.inner.save_backup(name, contents)
    }

    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.inner.save_journal(name, contents)
    }

    fn replace_journal(&mut self, id: &str, contents: &str) -> io::Result<PathBuf> {
        self.inner.replace_journal(id, contents)
    }

    fn remove_journal(&mut self, id: &str) -> io::Result<()> {
        self.inner.remove_journal(id)
    }

    fn journal_files(&mut self, now: Timestamp) -> io::Result<Vec<JournalFile>> {
        self.inner.journal_files(now)
    }

    fn journal_files_without_cleanup(&mut self) -> io::Result<Vec<JournalFile>> {
        self.inner.journal_files_without_cleanup()
    }

    fn lock_journal(&mut self) -> io::Result<Box<dyn JournalLock>> {
        self.inner.lock_journal()
    }

    fn try_lock_journal(&mut self) -> io::Result<Option<Box<dyn JournalLock>>> {
        self.inner.try_lock_journal()
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::{date, time};

    use super::*;
    use crate::app::journal;
    use crate::domain::{BackendError, Priority, TaskChange};
    use crate::infra::host::Scripted;
    use crate::tui::state::tests::{now, type_text};
    use crate::tui::state::{Action, Outcome, State};

    fn run<T>(host: &mut Scripted, test: impl FnOnce(&mut Session<'_>) -> T) -> T {
        let now = now();
        let config = Config::default();
        let backends = crate::infra::demo::backends(&now);
        let mut session = Session {
            now: &now,
            config: &config,
            backends: &backends,
            host,
        };
        test(&mut session)
    }

    fn delete(ids: &[&str]) -> Effect {
        Effect::Delete {
            ids: ids.iter().map(ToString::to_string).collect(),
            partial_confirmed: false,
        }
    }

    fn journal_action(host: &Scripted) -> String {
        let journal: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        journal["action"].as_str().unwrap().to_owned()
    }

    #[test]
    fn loading_names_lists_and_keeps_due_times() {
        let data = run(&mut Scripted::new(), |session| session.load(false).unwrap());
        assert!(!data.includes_completed);
        let passport = data
            .items
            .iter()
            .find(|item| item.id == "reminder-3")
            .unwrap();
        assert_eq!(passport.list.as_deref(), Some("Reminders"));
        assert_eq!(passport.due_time, Some(time(9, 0, 0, 0)));
        assert!(passport.due_at.is_some());
        assert_eq!(
            data.lists
                .iter()
                .map(|list| list.name.as_str())
                .collect::<Vec<_>>(),
            ["Reminders", "Work"]
        );
        let completed = run(&mut Scripted::new(), |session| session.load(true).unwrap());
        assert!(completed.items.iter().any(|item| item.completed));
    }

    #[test]
    fn add_uses_the_quick_add_parser() {
        let response = run(&mut Scripted::new(), |session| {
            session.perform(Effect::Add("Call mom tomorrow !high #Work".into()), false)
        });
        let Response::Added(item) = response else {
            panic!("expected an added reminder, got {response:?}");
        };
        assert_eq!(item.title, "Call mom");
        assert_eq!(item.due, Some(date(2026, 10, 31)));
        assert_eq!(item.priority, Some(Priority::High));
        assert_eq!(item.list.as_deref(), Some("Work"));
    }

    #[test]
    fn add_reports_quick_add_errors() {
        let response = run(&mut Scripted::new(), |session| {
            session.perform(Effect::Add("Pay !low !high".into()), false)
        });
        assert_eq!(
            response,
            Response::Failed(
                "inline priority is specified more than once; escape one with a leading backslash"
                    .into()
            )
        );
    }

    #[test]
    fn changes_run_through_the_one_step_commands_and_reload() {
        let mut host = Scripted::new();
        let response = run(&mut host, |session| {
            session.perform(
                Effect::Change {
                    change: TaskChange::Complete,
                    ids: vec!["reminder-2".into(), "reminder-4".into()],
                },
                false,
            )
        });
        let Response::Changed {
            message,
            error,
            data: Some(data),
        } = response
        else {
            panic!("expected a change, got {response:?}");
        };
        assert_eq!(
            message,
            "Completed \"Pay rent\"; Completed \"Buy groceries\""
        );
        assert!(!error);
        assert_eq!(data.items.len(), 4);
        assert_eq!(journal_action(&host), "task_done");
        assert_eq!(host.backups.len(), 1);
    }

    #[test]
    fn move_and_priority_report_what_changed() {
        let response = run(&mut Scripted::new(), |session| {
            session.perform(
                Effect::Change {
                    change: TaskChange::Move("demo-list-work".into()),
                    ids: vec!["reminder-2".into()],
                },
                false,
            )
        });
        assert!(
            matches!(&response, Response::Changed { message, .. } if message == "Moved \"Pay rent\" to Work"),
            "{response:?}"
        );
        let response = run(&mut Scripted::new(), |session| {
            session.perform(
                Effect::Change {
                    change: TaskChange::Priority(Some(Priority::High)),
                    ids: vec!["reminder-1".into()],
                },
                false,
            )
        });
        assert!(
            matches!(&response, Response::Changed { message, .. } if message == "No changes."),
            "{response:?}"
        );
    }

    #[test]
    fn delete_journals_a_task_delete() {
        let mut host = Scripted::new();
        let response = run(&mut host, |session| {
            session.perform(delete(&["reminder-2"]), true)
        });
        let Response::Changed {
            message,
            data: Some(data),
            ..
        } = response
        else {
            panic!("expected a change, got {response:?}");
        };
        assert_eq!(message, "Deleted \"Pay rent\"");
        assert!(data.includes_completed);
        assert_eq!(journal_action(&host), "task_delete");
    }

    #[test]
    fn a_journal_failure_after_apply_reports_the_change_and_reloads() {
        let mut host = Scripted::new();
        host.fail_next_journal_save();
        let response = run(&mut host, |session| {
            session.perform(delete(&["reminder-2"]), false)
        });
        let Response::Changed {
            message,
            error,
            data: Some(_),
        } = response
        else {
            panic!("expected a change, got {response:?}");
        };
        assert!(error);
        assert_eq!(
            message,
            format!("{} injected journal save failure", journal::UNRECORDED)
        );
        assert_eq!(host.journals, Vec::<(String, String)>::new());
        assert_eq!(host.backups.len(), 1);
    }

    struct ReloadFails {
        demo: Backends,
        snapshots: std::cell::Cell<usize>,
    }

    impl crate::domain::Reminders for ReloadFails {
        fn add(&self, reminder: crate::domain::NewReminder) -> Result<Reminder, BackendError> {
            self.demo.reminders.add(reminder)
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            self.demo.reminders.list()
        }

        fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            self.snapshots.set(self.snapshots.get() + 1);
            if self.snapshots.get() > 1 {
                return Err(BackendError::Unsupported {
                    app: "Reminders",
                    operation: "reloading",
                });
            }
            self.demo.reminders.snapshot(list)
        }

        fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
            self.demo.reminders.snapshot_including_completed()
        }

        fn apply(
            &self,
            changes: &[crate::domain::ReminderChange],
        ) -> Result<crate::domain::ApplyResult, BackendError> {
            self.demo.reminders.apply(changes)
        }

        fn reminder(
            &self,
            id: &str,
        ) -> Result<Option<crate::domain::StoredReminder>, BackendError> {
            self.demo.reminders.reminder(id)
        }
    }

    #[test]
    fn a_reload_failure_keeps_the_success_message() {
        let now = now();
        let config = Config::default();
        let demo = crate::infra::demo::backends(&now);
        let backends = Backends::new(
            || -> Box<dyn crate::domain::Notes> { unreachable!() },
            move || {
                Box::new(ReloadFails {
                    demo,
                    snapshots: std::cell::Cell::new(0),
                })
            },
            || -> Box<dyn crate::domain::Calendar> { unreachable!() },
        );
        let mut host = Scripted::new();
        let mut session = Session {
            now: &now,
            config: &config,
            backends: &backends,
            host: &mut host,
        };
        let response = session.perform(
            Effect::Change {
                change: TaskChange::Complete,
                ids: vec!["reminder-2".into()],
            },
            false,
        );
        assert_eq!(
            response,
            Response::Changed {
                message: "Completed \"Pay rent\" Couldn't reload reminders, so the list may be out of date: Reminders doesn't support reloading".into(),
                error: true,
                data: None,
            }
        );
        assert_eq!(journal_action(&host), "task_done");
    }

    struct GainsAlarm {
        demo: Backends,
        snapshots: std::cell::Cell<usize>,
    }

    impl GainsAlarm {
        fn mark(&self, mut snapshot: ReminderSnapshot) -> ReminderSnapshot {
            if self.snapshots.get() > 1 {
                for reminder in &mut snapshot.reminders {
                    reminder.backup_partial |= reminder.id == "reminder-2";
                }
            }
            snapshot
        }
    }

    impl crate::domain::Reminders for GainsAlarm {
        fn add(&self, reminder: crate::domain::NewReminder) -> Result<Reminder, BackendError> {
            self.demo.reminders.add(reminder)
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            self.demo.reminders.list()
        }

        fn snapshot(&self, list: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            self.snapshots.set(self.snapshots.get() + 1);
            Ok(self.mark(self.demo.reminders.snapshot(list)?))
        }

        fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
            self.snapshots.set(self.snapshots.get() + 1);
            Ok(self.mark(self.demo.reminders.snapshot_including_completed()?))
        }

        fn apply(
            &self,
            changes: &[crate::domain::ReminderChange],
        ) -> Result<crate::domain::ApplyResult, BackendError> {
            self.demo.reminders.apply(changes)
        }

        fn reminder(
            &self,
            id: &str,
        ) -> Result<Option<crate::domain::StoredReminder>, BackendError> {
            self.demo.reminders.reminder(id)
        }
    }

    #[test]
    fn a_reminder_that_gains_an_alarm_after_loading_needs_the_count_typed() {
        let now = now();
        let config = Config::default();
        let demo = crate::infra::demo::backends(&now);
        let backends = Backends::new(
            || -> Box<dyn crate::domain::Notes> { unreachable!() },
            move || {
                Box::new(GainsAlarm {
                    demo,
                    snapshots: std::cell::Cell::new(0),
                })
            },
            || -> Box<dyn crate::domain::Calendar> { unreachable!() },
        );
        let mut host = Scripted::new();
        let mut session = Session {
            now: &now,
            config: &config,
            backends: &backends,
            host: &mut host,
        };
        let (mut state, _) = State::new(session.load(false).unwrap(), "rent", now.clone(), true);
        assert!(!state.item("reminder-2").unwrap().backup_partial);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        let effect = state.update(Action::Hotkey('y')).unwrap();
        let response = session.perform(effect, false);
        let Response::DeleteRefused {
            partial,
            data: Some(_),
            ..
        } = &response
        else {
            panic!("expected a refusal with fresh data, got {response:?}");
        };
        assert_eq!(partial, &["reminder-2"]);
        assert_eq!(state.resolve(response), None);
        assert_eq!(
            state.status(),
            Some(&crate::tui::state::Status {
                text: "\"Pay rent\" now has alarms, repeats, or links that undo can't bring back; type the count to delete it".into(),
                error: true,
            })
        );
        assert!(state.item("reminder-2").unwrap().backup_partial);
        assert_eq!(state.mode(), crate::tui::state::Mode::ConfirmDelete);
        assert!(state.delete_needs_count());
        assert_eq!(host.backups, Vec::<(String, String)>::new());
        assert_eq!(host.journals, Vec::<(String, String)>::new());

        let mut session = Session {
            now: &now,
            config: &config,
            backends: &backends,
            host: &mut host,
        };
        state.update(Action::Hotkey('1'));
        let effect = state.update(Action::Accept).unwrap();
        assert_eq!(
            effect,
            Effect::Delete {
                ids: vec!["reminder-2".into()],
                partial_confirmed: true,
            }
        );
        let response = session.perform(effect, false);
        assert!(
            matches!(&response, Response::Changed { message, error: false, data: Some(_) } if message == "Deleted \"Pay rent\""),
            "{response:?}"
        );
        assert_eq!(journal_action(&host), "task_delete");
        assert!(
            host.backups[0].1.contains("\"partial\": true"),
            "{}",
            host.backups[0].1
        );
    }

    #[test]
    fn loading_copies_partial_backups_into_items() {
        let now = now();
        let mut snapshot = crate::infra::demo::backends(&now)
            .reminders
            .snapshot(None)
            .unwrap();
        snapshot.reminders[0].backup_partial = true;
        let first = snapshot.reminders[0].id.clone();
        let data = data(snapshot, false, &now);
        assert!(
            data.items
                .iter()
                .find(|item| item.id == first)
                .unwrap()
                .backup_partial
        );
        assert!(
            data.items
                .iter()
                .filter(|item| item.id != first)
                .all(|item| !item.backup_partial)
        );
    }

    #[test]
    fn failures_come_back_as_messages() {
        let mut host = Scripted::new();
        let response = run(&mut host, |session| {
            session.perform(
                Effect::Change {
                    change: TaskChange::Complete,
                    ids: vec!["reminder-6".into()],
                },
                false,
            )
        });
        assert_eq!(
            response,
            Response::Failed("\"Call the dentist\" is already completed".into())
        );
        assert_eq!(host.journals, Vec::<(String, String)>::new());
    }

    #[test]
    fn the_act_menu_drives_a_change_end_to_end() {
        let mut host = Scripted::new();
        run(&mut host, |session| {
            let (mut state, _) = State::new(session.load(false).unwrap(), "", now(), true);
            type_text(&mut state, "rent");
            state.update(Action::OpenMenu);
            let effect = state.update(Action::Hotkey('x')).unwrap();
            let response = session.perform(effect, state.includes_completed());
            assert_eq!(state.resolve(response), None);
            assert_eq!(
                state.status().map(|status| status.text.as_str()),
                Some("Completed \"Pay rent\"")
            );

            state.update(Action::KillToStart);
            type_text(&mut state, "Water plants");
            state.update(Action::Down);
            let effect = state.update(Action::Accept).unwrap();
            let response = session.perform(effect, false);
            state.resolve(response);
            assert_eq!(
                state.outcome(),
                Some(&Outcome::Picked(vec!["reminder-5".into()]))
            );
        });
        assert_eq!(journal_action(&host), "task_done");
    }
}

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::{Timestamp, Zoned};
use miette::Result;

use super::state::{Data, Effect, Item, ListChoice, Response};
use crate::app::commands::reminder_from_snapshot;
use crate::app::{journal, quick};
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
            Effect::Delete(ids) => {
                let (now, backends) = (self.now, self.backends);
                self.quietly(|host| {
                    let deleted = quick::delete(&ids, now, backends, host)?;
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
            Err(error) => Response::Failed(clean(&error.to_string())),
        }
    }

    fn reloaded(&self, message: String, error: bool, includes_completed: bool) -> Response {
        match self.load(includes_completed) {
            Ok(data) => Response::Changed {
                message,
                error,
                data: Some(data),
            },
            Err(reload) => Response::Changed {
                message: format!(
                    "{message} Couldn't reload reminders, so the list may be out of date: {}",
                    clean(&reload.to_string())
                ),
                error: true,
                data: None,
            },
        }
    }

    fn quietly(&mut self, run: impl FnOnce(&mut dyn Host) -> Result<String>) -> Result<Response> {
        let mut host = Quiet {
            inner: &mut *self.host,
            said: Vec::new(),
        };
        let result = run(&mut host);
        let with_said = |text: String| {
            std::iter::once(text)
                .chain(host.said.iter().map(|line| clean(line)))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let applied = host.said.iter().any(|line| line == journal::UNRECORDED);
        match result {
            Ok(summary) => Ok(Response::Changed {
                message: with_said(summary),
                error: false,
                data: None,
            }),
            Err(error) if applied => Ok(Response::Changed {
                message: with_said(error.to_string()),
                error: true,
                data: None,
            }),
            Err(error) => Err(miette::miette!("{}", with_said(error.to_string()))),
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
    }
}

struct Quiet<'a> {
    inner: &'a mut dyn Host,
    said: Vec<String>,
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
            session.perform(Effect::Delete(vec!["reminder-2".into()]), true)
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
            session.perform(Effect::Delete(vec!["reminder-2".into()]), false)
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
        assert!(message.ends_with(journal::UNRECORDED), "{message}");
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
            let (mut state, _) = State::new(session.load(false).unwrap(), "", now());
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

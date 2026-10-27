use jiff::Zoned;
use miette::{IntoDiagnostic, Result};

use crate::app::{edit, journal, markdown};
use crate::cli::output::clean;
use crate::domain::{Backends, ChangeCounts, ReminderChange};
use crate::infra::host::Host;

pub fn run(
    entry: Option<&str>,
    now: &Zoned,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<Option<(String, ChangeCounts)>> {
    let (id, changes, counts, partial) = journal::undo_changes(entry, host)?;
    host.say(&clean(&describe(&id, &changes, partial)));
    if !confirmed(host, &changes)? {
        return Ok(None);
    }
    let contents = edit::backup_changes(&changes)?.unwrap_or_else(|| "[]".into());
    let name = format!("{}-undo.json", now.strftime("%Y%m%dT%H%M%S"));
    let saved = host.save_backup(&name, &contents).into_diagnostic()?;
    host.say(&format!(
        "Backed up the reminders being changed to {}",
        saved.display()
    ));
    let applied = backends.reminders.apply(&changes)?;
    if let Err(error) = journal::record_undo(now, &id, &changes, &applied, host) {
        host.say("The undo was applied, but nib couldn't record the journal entry.");
        return Err(error);
    }
    Ok(Some((id, counts)))
}

fn describe(id: &str, changes: &[ReminderChange], partial: bool) -> String {
    let mut lines = vec![format!("Undo {id}:")];
    for change in changes {
        lines.push(match change {
            ReminderChange::Add { title, .. } => format!("  + add       \"{title}\""),
            ReminderChange::Restore { before } => {
                format!(
                    "  + re-create \"{}\" with a new identifier",
                    markdown::normalize_title(&before.title)
                )
            }
            ReminderChange::Update { before, edit } if edit.completed == Some(false) => {
                format!(
                    "  ○ incomplete \"{}\"",
                    markdown::normalize_title(&before.title)
                )
            }
            ReminderChange::Update { before, .. } => {
                format!(
                    "  ~ restore   \"{}\"",
                    markdown::normalize_title(&before.title)
                )
            }
            ReminderChange::Delete { before } => {
                format!(
                    "  - delete    added reminder \"{}\"",
                    markdown::normalize_title(&before.title)
                )
            }
        });
    }
    if changes
        .iter()
        .any(|change| matches!(change, ReminderChange::Restore { .. }))
    {
        lines.push(
            "  ! Re-created reminders get new identifiers and lose anything the backup couldn't capture."
                .into(),
        );
    }
    if partial {
        lines.push("  ! At least one deleted reminder has a partial backup.".into());
    }
    lines.join("\n")
}

fn confirmed(host: &mut dyn Host, changes: &[ReminderChange]) -> Result<bool> {
    let deletions = changes
        .iter()
        .filter(|change| matches!(change, ReminderChange::Delete { .. }))
        .count();
    if deletions > 3 {
        let answer = host
            .ask(&format!(
                "This deletes {deletions} reminders. Type {deletions} to undo: "
            ))
            .into_diagnostic()?;
        return Ok(answer == deletions.to_string());
    }
    let answer = host
        .ask(&format!(
            "Apply {}? [y/N] ",
            count(changes.len(), "undo change")
        ))
        .into_diagnostic()?;
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::app::journal;
    use crate::domain::{
        ApplyResult, BackendError, Calendar, NewReminder, Notes, Reminder, ReminderEdit,
        ReminderSnapshot, Reminders, StoredReminder,
    };
    use crate::infra::host::Scripted;

    struct State {
        reminder: Rc<RefCell<StoredReminder>>,
    }

    impl Reminders for State {
        fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
            unreachable!()
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            unreachable!()
        }

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            unreachable!()
        }

        fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
            let [ReminderChange::Update { before, edit }] = changes else {
                unreachable!()
            };
            if *self.reminder.borrow() != *before {
                return Err(BackendError::Conflict {
                    titles: vec![format!("\"{}\"", before.title)],
                });
            }
            let mut written = before.clone();
            if let Some(completed) = edit.completed {
                written.completed = completed;
            }
            self.reminder.replace(written.clone());
            Ok(ApplyResult {
                written: vec![Some(written)],
            })
        }
    }

    fn now() -> Zoned {
        date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn reminder(completed: bool) -> StoredReminder {
        StoredReminder {
            id: "reminder-1".into(),
            list_id: "list-1".into(),
            title: "Finish report".into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            completed,
            modified: None,
            alarms_present: false,
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    fn setup(conflict: bool) -> (Backends, Rc<RefCell<StoredReminder>>, Scripted) {
        let before = reminder(false);
        let written = reminder(true);
        let change = ReminderChange::Update {
            before,
            edit: ReminderEdit {
                completed: Some(true),
                ..ReminderEdit::default()
            },
        };
        let mut host = Scripted::new().answer("y");
        journal::record_edit(
            &now(),
            &[change],
            &ApplyResult {
                written: vec![Some(written.clone())],
            },
            &mut host,
        )
        .unwrap();
        let state = Rc::new(RefCell::new(written));
        if conflict {
            state.borrow_mut().title = "Changed elsewhere".into();
        }
        let backend_state = state.clone();
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || {
                Box::new(State {
                    reminder: backend_state,
                })
            },
            || -> Box<dyn Calendar> { unreachable!() },
        );
        (backends, state, host)
    }

    #[test]
    fn undo_confirms_backs_up_applies_and_journals() {
        let (backends, state, mut host) = setup(false);
        let result = run(None, &now(), &backends, &mut host).unwrap().unwrap();
        assert_eq!(result.0, "20261026T090000-journal");
        assert!(!state.borrow().completed);
        assert_eq!(host.backups.len(), 1);
        assert_eq!(host.backups[0].0, "20261026T090000-undo.json");
        assert_eq!(host.journals.len(), 2);
        assert!(host.journals[1].1.contains("\"action\": \"undo\""));
        assert!(host.said.iter().any(|line| line.contains("○ incomplete")));
    }

    #[test]
    fn undo_stops_all_changes_on_a_conflict() {
        let (backends, state, mut host) = setup(true);
        let result = run(None, &now(), &backends, &mut host);
        assert!(result.unwrap_err().to_string().contains("Finish report"));
        assert!(state.borrow().completed);
        assert_eq!(host.journals.len(), 1);
    }
}

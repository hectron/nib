use jiff::Zoned;
use miette::{IntoDiagnostic, Result, miette};

use crate::app::{edit, journal, markdown};
use crate::cli::output::clean_lines;
use crate::domain::{Backends, ChangeCounts, ReminderChange};
use crate::infra::host::Host;

#[derive(Debug)]
pub enum Outcome {
    Cancelled {
        id: String,
    },
    Applied {
        id: String,
        undo_id: String,
        counts: ChangeCounts,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Applied,
    Removed,
    Cancelled,
}

pub fn run(
    entry: Option<&str>,
    now: &Zoned,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<Outcome> {
    let lock = host.lock_journal().into_diagnostic()?;
    let plan = journal::undo_changes(entry, now.timestamp(), host)?;
    drop(lock);
    host.say(&clean_lines(&describe(
        &plan.id,
        &plan.changes,
        plan.partial,
    )));
    if !confirmed(host, &plan.changes)? {
        return Ok(Outcome::Cancelled { id: plan.id });
    }
    let _lock = host.lock_journal().into_diagnostic()?;
    journal::recheck_undo(&plan, now.timestamp(), host)?;
    let contents = edit::backup_changes(&plan.changes)?.unwrap_or_else(|| "[]".into());
    let name = format!("{}-undo.json", now.strftime("%Y%m%dT%H%M%S"));
    let saved = host.save_backup(&name, &contents).into_diagnostic()?;
    host.say(&format!(
        "Backed up the reminders being changed to {}",
        saved.display()
    ));
    let pending_id = journal::begin_undo(now, &plan.id, &plan.changes, host)?;
    let applied = match backends.reminders.apply(&plan.changes) {
        Ok(applied) => applied,
        Err(error) => {
            if let Err(removal_error) = host.remove_journal(&pending_id) {
                return Err(miette::miette!(
                    "{error}; additionally, nib couldn't remove pending journal entry {pending_id:?}: {removal_error}"
                ));
            }
            return Err(error.into());
        }
    };
    if let Err(error) =
        journal::finish_undo(&pending_id, now, &plan.id, &plan.changes, &applied, host)
    {
        host.say("The undo was applied, but its journal entry is still marked pending.");
        return Err(error);
    }
    Ok(Outcome::Applied {
        id: plan.id,
        undo_id: pending_id,
        counts: plan.counts,
    })
}

pub fn resolve(id: &str, now: &Zoned, host: &mut dyn Host) -> Result<Resolution> {
    let lock = host.lock_journal().into_diagnostic()?;
    let expected = journal::pending_undo(id, now.timestamp(), host)?;
    drop(lock);
    let mut lines = vec![format!(
        "Pending undo {id} was interrupted. If it was applied, Reminders shows:"
    )];
    lines.extend(expected.lines.iter().map(|line| format!("  - {line}")));
    host.say(&clean_lines(&lines.join("\n")));
    let answer = host
        .ask("Type `applied` if Reminders matches, `absent` if none of it happened, or anything else to cancel: ")
        .into_diagnostic()?;
    let resolution = match answer.trim().to_ascii_lowercase().as_str() {
        "applied" => Resolution::Applied,
        "absent" => Resolution::Removed,
        _ => return Ok(Resolution::Cancelled),
    };
    let _lock = host.lock_journal().into_diagnostic()?;
    if journal::pending_undo(id, now.timestamp(), host)? != expected {
        return Err(miette!(
            "pending undo {id:?} changed while nib waited for the answer; nothing was changed"
        ));
    }
    journal::resolve_pending(id, resolution == Resolution::Applied, now.timestamp(), host)?;
    Ok(resolution)
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
            ReminderChange::Update { before, edit }
                if edit.completed == Some(false) && reverted_fields(edit) == ["completion"] =>
            {
                format!(
                    "  ○ incomplete \"{}\"",
                    markdown::normalize_title(&before.title)
                )
            }
            ReminderChange::Update { before, edit } => {
                format!(
                    "  ~ restore   \"{}\" ({})",
                    markdown::normalize_title(&before.title),
                    reverted_fields(edit).join(", ")
                )
            }
            ReminderChange::Delete { before } => {
                format!(
                    "  - delete    re-created reminder \"{}\"",
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

fn reverted_fields(edit: &crate::domain::ReminderEdit) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if edit.title.is_some() {
        fields.push("title");
    }
    if edit.notes.is_changed() {
        fields.push("notes");
    }
    if edit.due.is_changed() || edit.due_components.is_some() {
        fields.push("due date");
    }
    if edit.priority.is_changed() {
        fields.push("priority");
    }
    if edit.list_id.is_some() {
        fields.push("list");
    }
    if edit.completed.is_some() {
        fields.push("completion");
    }
    fields
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
    use std::cell::Cell;
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
        unrecorded: bool,
        journal_locks: Rc<Cell<usize>>,
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
            assert!(
                self.journal_locks.get() > 0,
                "apply requires the journal lock"
            );
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
                unrecorded: vec![self.unrecorded],
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
            absolute_alarms: Some(Vec::new()),
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    fn setup(
        conflict: bool,
        unrecorded: bool,
    ) -> (Backends, Rc<RefCell<StoredReminder>>, Scripted) {
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
                unrecorded: Vec::new(),
            },
            &mut host,
        )
        .unwrap();
        let state = Rc::new(RefCell::new(written));
        if conflict {
            state.borrow_mut().title = "Changed elsewhere".into();
        }
        let backend_state = state.clone();
        let journal_locks = host.journal_lock_state();
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || {
                Box::new(State {
                    reminder: backend_state,
                    unrecorded,
                    journal_locks,
                })
            },
            || -> Box<dyn Calendar> { unreachable!() },
        );
        (backends, state, host)
    }

    #[test]
    fn undo_confirms_backs_up_applies_and_journals() {
        let (backends, state, mut host) = setup(false, false);
        let Outcome::Applied { id, .. } = run(None, &now(), &backends, &mut host).unwrap() else {
            panic!("undo should be applied");
        };
        assert_eq!(id, "20261026T090000-journal");
        assert!(!state.borrow().completed);
        assert_eq!(host.backups.len(), 1);
        assert_eq!(host.backups[0].0, "20261026T090000-undo.json");
        assert_eq!(host.journals.len(), 2);
        assert!(host.journals[1].1.contains("\"action\": \"undo\""));
        assert!(host.said.iter().any(|line| line.contains("○ incomplete")));
        assert_eq!(host.journal_lock_count, 2);
    }

    #[test]
    fn undo_releases_the_lock_for_confirmation_and_rechecks_the_entry() {
        let (backends, state, mut host) = setup(false, false);
        let id = host.journals[0].0.strip_suffix(".json").unwrap().to_owned();
        let changed = format!("{}\n", host.journals[0].1);
        host = host.replace_journal_on_answer(&id, &changed);

        let error = run(None, &now(), &backends, &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("changed while nib waited for confirmation"));
        assert!(state.borrow().completed);
        assert_eq!(host.backups, Vec::<(String, String)>::new());
        assert_eq!(host.journals.len(), 1);
        assert_eq!(host.journal_lock_count, 2);
    }

    #[test]
    fn undo_refuses_an_unrecorded_reminder_before_prompting() {
        let (backends, state, mut host) = setup(false, false);
        let mut entry: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        entry["changes"][0]["written"]["unrecorded"] = serde_json::json!(true);
        host.journals[0].1 = serde_json::to_string_pretty(&entry).unwrap();

        let error = run(None, &now(), &backends, &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("couldn't re-read 1 reminder after saving"));
        assert!(error.contains("\"Finish report\""));
        assert!(state.borrow().completed);
        assert_eq!(host.backups, Vec::<(String, String)>::new());
        assert_eq!(host.journal_lock_count, 1);
    }

    #[test]
    fn undo_stops_all_changes_on_a_conflict() {
        let (backends, state, mut host) = setup(true, false);
        let result = run(None, &now(), &backends, &mut host);
        assert!(result.unwrap_err().to_string().contains("Finish report"));
        assert!(state.borrow().completed);
        assert_eq!(host.journals.len(), 1);
    }

    #[test]
    fn failed_final_journal_save_leaves_pending_marker_and_refuses_second_undo() {
        let (backends, state, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        let first = run(None, &now(), &backends, &mut host).unwrap_err();
        assert!(first.to_string().contains("injected journal save failure"));
        assert!(!state.borrow().completed);
        assert_eq!(host.journals.len(), 2);

        let second = run(None, &now(), &backends, &mut host).unwrap_err();
        let message = second.to_string();
        assert!(message.contains("is pending"));
        assert!(message.contains("/backups/20261026T090000-journal-test2.json"));
    }

    #[test]
    fn post_commit_recording_problem_finishes_the_pending_undo() {
        let (backends, state, mut host) = setup(false, true);

        let outcome = run(None, &now(), &backends, &mut host).unwrap();

        assert!(matches!(outcome, Outcome::Applied { .. }));
        assert!(!state.borrow().completed);
        assert!(host.journals[1].1.contains("\"status\": \"applied\""));
        assert!(host.journals[1].1.contains("\"unrecorded\": true"));
        assert!(host.said.iter().any(|line| line
            == "warning: nib couldn't re-read 1 reminder after saving; undo of this entry is limited"));
    }

    #[test]
    fn apply_and_pending_removal_errors_are_both_reported() {
        let (backends, _, mut host) = setup(true, false);
        host.fail_next_journal_remove();

        let error = run(None, &now(), &backends, &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("Finish report"));
        assert!(error.contains("injected journal removal failure"));
    }

    #[test]
    fn resolving_a_pending_undo_marks_it_applied_or_removes_it() {
        let (backends, _, mut applied_host) = setup(false, false);
        applied_host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut applied_host).unwrap_err();
        applied_host = applied_host.answer("applied");
        let pending_id = applied_host.journals[1]
            .0
            .strip_suffix(".json")
            .unwrap()
            .to_owned();
        let pending_summaries = journal::log(now().timestamp(), &mut applied_host).unwrap();
        assert_eq!(
            pending_summaries[0].status,
            crate::domain::JournalStatus::Pending
        );
        assert!(!pending_summaries[1].undone);

        assert_eq!(
            resolve(&pending_id, &now(), &mut applied_host).unwrap(),
            Resolution::Applied
        );
        let summaries = journal::log(now().timestamp(), &mut applied_host).unwrap();
        assert_eq!(summaries[0].status, crate::domain::JournalStatus::Applied);
        assert!(summaries[1].undone);
        let error = journal::undo_changes(Some(&pending_id), now().timestamp(), &mut applied_host)
            .unwrap_err()
            .to_string();
        assert!(error.contains("can't be reversed"));

        let (backends, _, mut removed_host) = setup(false, false);
        removed_host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut removed_host).unwrap_err();
        removed_host = removed_host.answer("absent");
        let pending_id = removed_host.journals[1]
            .0
            .strip_suffix(".json")
            .unwrap()
            .to_owned();

        assert_eq!(
            resolve(&pending_id, &now(), &mut removed_host).unwrap(),
            Resolution::Removed
        );
        assert_eq!(removed_host.journals.len(), 1);
    }

    #[test]
    fn resolve_rechecks_the_pending_entry_after_the_answer() {
        let (backends, _, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut host).unwrap_err();
        let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
        let changed = host.journals[1]
            .1
            .replace("Finish report", "Renamed report");
        host = host
            .replace_journal_on_answer(&pending_id, &changed)
            .answer("applied");

        let error = resolve(&pending_id, &now(), &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("changed while nib waited for the answer"));
        assert!(host.journals[1].1.contains("\"status\": \"pending\""));
        assert_eq!(host.journal_lock_count, 4);
    }

    #[test]
    fn resolve_rejects_a_change_outside_the_summary_lines() {
        let (backends, _, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut host).unwrap_err();
        let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
        let original = host.journals[1].1.clone();
        let changed = original.replacen("\"id\": \"reminder-1\"", "\"id\": \"reminder-2\"", 1);
        assert_ne!(changed, original);
        assert!(changed.find("\"id\": \"reminder-2\"") < changed.find("\"changes\""));
        host = host
            .replace_journal_on_answer(&pending_id, &changed)
            .answer("applied");

        let error = resolve(&pending_id, &now(), &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("changed while nib waited for the answer"));
        assert_eq!(host.journals[1].1, changed);
    }

    #[test]
    fn resolve_reports_the_read_error_for_an_entry_unreadable_while_waiting() {
        let (backends, _, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut host).unwrap_err();
        let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
        let before = host.journals.clone();
        host = host
            .unreadable_journal_on_answer(&pending_id, "permission denied")
            .answer("applied");

        let error = resolve(&pending_id, &now(), &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("couldn't read journal entry"));
        assert!(error.contains("permission denied"));
        assert!(!error.contains("changed while"));
        assert_eq!(host.journals, before);
    }

    #[test]
    fn resolve_reports_an_entry_resolved_while_waiting_for_the_answer() {
        let (backends, _, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut host).unwrap_err();
        let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
        let resolved = host.journals[1]
            .1
            .replace("\"status\": \"pending\"", "\"status\": \"applied\"");
        host = host
            .replace_journal_on_answer(&pending_id, &resolved)
            .answer("absent");

        let error = resolve(&pending_id, &now(), &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("is not pending"));
        assert!(!error.contains("changed while"));
        assert_eq!(host.journals.len(), 2);
        assert_eq!(host.journals[1].1, resolved);
    }

    #[test]
    fn resolve_reports_an_entry_removed_while_waiting_for_the_answer() {
        let (backends, _, mut host) = setup(false, false);
        host.fail_next_journal_replace();
        run(None, &now(), &backends, &mut host).unwrap_err();
        let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
        let original = host.journals[0].clone();
        host = host.remove_journal_on_answer(&pending_id).answer("applied");

        let error = resolve(&pending_id, &now(), &mut host)
            .unwrap_err()
            .to_string();

        assert!(error.contains("not found"));
        assert!(!error.contains("changed while"));
        assert_eq!(host.journals, vec![original]);
    }

    #[test]
    fn resolve_changes_nothing_without_an_explicit_answer() {
        for answer in ["", "n", "y", "yes"] {
            let (backends, _, mut host) = setup(false, false);
            host.fail_next_journal_replace();
            run(None, &now(), &backends, &mut host).unwrap_err();
            host = host.answer(answer);
            let pending_id = host.journals[1].0.strip_suffix(".json").unwrap().to_owned();
            let before = host.journals.clone();

            assert_eq!(
                resolve(&pending_id, &now(), &mut host).unwrap(),
                Resolution::Cancelled,
                "{answer:?}"
            );
            assert_eq!(host.journals, before, "{answer:?}");
        }
    }

    #[test]
    fn preview_keeps_lines_and_names_fields_being_restored() {
        let before = reminder(false);
        let changes = vec![
            ReminderChange::Update {
                before: before.clone(),
                edit: ReminderEdit {
                    notes: crate::domain::FieldEdit::Remove,
                    due: crate::domain::FieldEdit::Set(date(2026, 10, 27)),
                    list_id: Some("list-2".into()),
                    ..ReminderEdit::default()
                },
            },
            ReminderChange::Delete { before },
        ];
        let preview = clean_lines(&describe("entry-1", &changes, false));
        assert!(preview.contains('\n'));
        assert!(preview.contains("notes, due date, list"));
        assert!(preview.contains("re-created reminder"));
    }
}

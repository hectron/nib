use std::collections::HashSet;

use jiff::Zoned;
use miette::{Diagnostic, IntoDiagnostic, Result};
use thiserror::Error;

use crate::app::{edit, journal};
use crate::cli::output::clean;
use crate::domain::{
    BackendError, Backends, FieldEdit, ReminderChange, ReminderEdit, ReminderList, StoredReminder,
    TaskChange, TaskChangeRequest,
};
use crate::infra::host::Host;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub reminder: StoredReminder,
    pub list: String,
    pub change: TaskChange,
}

#[derive(Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum SelectError {
    #[error("no {state} reminder matches \"{selector}\"{scope}")]
    #[diagnostic(
        code(nib::task::no_match),
        help("check the spelling, or pass the reminder's id from --json output")
    )]
    NoMatch {
        selector: String,
        state: &'static str,
        scope: String,
    },

    #[error(
        "\"{selector}\" matches {count} {state} reminders:\n{lines}",
        count = .candidates.len(),
        lines = .candidates.join("\n")
    )]
    #[diagnostic(
        code(nib::task::ambiguous),
        help("use more of the title, narrow with --list, or pass one of the ids")
    )]
    Ambiguous {
        selector: String,
        state: &'static str,
        candidates: Vec<String>,
    },

    #[error("\"{title}\" is already completed")]
    #[diagnostic(
        code(nib::task::wrong_state),
        help("use `nib task reopen` to mark it open again")
    )]
    AlreadyCompleted { title: String },

    #[error("\"{title}\" isn't completed")]
    #[diagnostic(
        code(nib::task::wrong_state),
        help("only completed reminders can be reopened")
    )]
    NotCompleted { title: String },

    #[error("reminder list \"{name}\" not found")]
    #[diagnostic(
        code(nib::task::list_not_found),
        help("run `nib task lists` to see list names")
    )]
    ListNotFound { name: String },

    #[error("more than one reminder list is named \"{name}\" ({ids})", ids = .ids.join(", "))]
    #[diagnostic(
        code(nib::task::ambiguous_list),
        help("pass the list id from `nib task lists --json` instead")
    )]
    AmbiguousList { name: String, ids: Vec<String> },

    #[error("reminder list \"{name}\" is read-only")]
    #[diagnostic(code(nib::task::read_only_list), help("choose a list you can change"))]
    ReadOnlyList { name: String },
}

pub fn run(
    request: &TaskChangeRequest,
    now: &Zoned,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<Vec<Changed>> {
    let TaskChangeRequest {
        change,
        selectors,
        list,
    } = request;
    let reopen = *change == TaskChange::Reopen;
    let snapshot = if reopen {
        backends.reminders.snapshot_including_completed()?
    } else {
        backends.reminders.snapshot(None)?
    };
    let selected = select(
        change,
        selectors,
        list.as_deref(),
        &snapshot.lists,
        &snapshot.reminders,
        |id| {
            if reopen {
                Ok(None)
            } else {
                backends.reminders.reminder(id)
            }
        },
    )?;
    let changes = plan(change, &selected, &snapshot.lists)?;
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let _lock = host.lock_journal().into_diagnostic()?;
    edit::back_up(now, &changes, host)?;
    let applied = backends.reminders.apply(&changes)?;
    if let Err(error) =
        journal::record_action(now, change.journal_action(), &changes, &applied, host)
    {
        host.say("The changes were applied, but nib couldn't record the journal entry.");
        return Err(error);
    }
    Ok(changes
        .iter()
        .zip(&applied.written)
        .filter_map(|(planned, written)| {
            let ReminderChange::Update { before, edit } = planned else {
                return None;
            };
            let reminder = written.clone().unwrap_or_else(|| expected(before, edit));
            Some(Changed {
                list: list_name(&snapshot.lists, &reminder.list_id),
                reminder,
                change: change.clone(),
            })
        })
        .collect())
}

pub(crate) fn select(
    change: &TaskChange,
    selectors: &[String],
    list: Option<&str>,
    lists: &[ReminderList],
    known: &[StoredReminder],
    mut fetch: impl FnMut(&str) -> std::result::Result<Option<StoredReminder>, BackendError>,
) -> Result<Vec<StoredReminder>> {
    let reopen = *change == TaskChange::Reopen;
    let state = if reopen { "completed" } else { "open" };
    let scope_ids = list
        .map(|name| {
            let ids: HashSet<&str> = lists
                .iter()
                .filter(|candidate| candidate.name == name)
                .map(|candidate| candidate.id.as_str())
                .collect();
            if ids.is_empty() {
                Err(SelectError::ListNotFound { name: clean(name) })
            } else {
                Ok(ids)
            }
        })
        .transpose()?;
    let scope = list
        .map(|name| format!(" in \"{}\"", clean(name)))
        .unwrap_or_default();
    let mut selected: Vec<StoredReminder> = Vec::new();
    for selector in selectors {
        let reminder = if let Some(found) = known.iter().find(|reminder| reminder.id == *selector) {
            check_state(change, found)?;
            found.clone()
        } else {
            let needle = selector.to_lowercase();
            let matches: Vec<&StoredReminder> = known
                .iter()
                .filter(|reminder| {
                    reminder.completed == reopen
                        && scope_ids
                            .as_ref()
                            .is_none_or(|ids| ids.contains(reminder.list_id.as_str()))
                        && reminder.title.to_lowercase().contains(&needle)
                })
                .collect();
            match matches.as_slice() {
                [only] => (*only).clone(),
                candidates @ [_, _, ..] => {
                    return Err(SelectError::Ambiguous {
                        selector: clean(selector),
                        state,
                        candidates: candidates
                            .iter()
                            .map(|reminder| {
                                format!(
                                    "  {}  \"{}\"  ({})",
                                    clean(&reminder.id),
                                    clean(&reminder.title),
                                    clean(&list_name(lists, &reminder.list_id))
                                )
                            })
                            .collect(),
                    }
                    .into());
                }
                [] => {
                    if let Some(found) = fetch(selector)? {
                        check_state(change, &found)?;
                        found
                    } else {
                        return Err(SelectError::NoMatch {
                            selector: clean(selector),
                            state,
                            scope,
                        }
                        .into());
                    }
                }
            }
        };
        if !selected.iter().any(|existing| existing.id == reminder.id) {
            selected.push(reminder);
        }
    }
    Ok(selected)
}

fn check_state(change: &TaskChange, reminder: &StoredReminder) -> Result<(), SelectError> {
    match change {
        TaskChange::Complete if reminder.completed => Err(SelectError::AlreadyCompleted {
            title: clean(&reminder.title),
        }),
        TaskChange::Reopen if !reminder.completed => Err(SelectError::NotCompleted {
            title: clean(&reminder.title),
        }),
        _ => Ok(()),
    }
}

pub(crate) fn plan(
    change: &TaskChange,
    selected: &[StoredReminder],
    lists: &[ReminderList],
) -> Result<Vec<ReminderChange>> {
    let target = match change {
        TaskChange::Move(name) => Some(target_list(name, lists)?),
        _ => None,
    };
    Ok(selected
        .iter()
        .filter_map(|reminder| {
            let edit = match change {
                TaskChange::Complete => ReminderEdit {
                    completed: Some(true),
                    ..ReminderEdit::default()
                },
                TaskChange::Reopen => ReminderEdit {
                    completed: Some(false),
                    ..ReminderEdit::default()
                },
                TaskChange::Priority(priority) if reminder.priority == *priority => return None,
                TaskChange::Priority(priority) => ReminderEdit {
                    priority: priority.map_or(FieldEdit::Remove, FieldEdit::Set),
                    ..ReminderEdit::default()
                },
                TaskChange::Move(_) => {
                    let target = target?;
                    if reminder.list_id == target.id {
                        return None;
                    }
                    ReminderEdit {
                        list_id: Some(target.id.clone()),
                        ..ReminderEdit::default()
                    }
                }
            };
            Some(ReminderChange::Update {
                before: reminder.clone(),
                edit,
            })
        })
        .collect())
}

fn target_list<'a>(name: &str, lists: &'a [ReminderList]) -> Result<&'a ReminderList> {
    let named: Vec<&ReminderList> = lists.iter().filter(|list| list.name == name).collect();
    let list = match named.as_slice() {
        [only] => *only,
        [] => lists
            .iter()
            .find(|list| list.id == name)
            .ok_or_else(|| SelectError::ListNotFound { name: clean(name) })?,
        _ => {
            return Err(SelectError::AmbiguousList {
                name: clean(name),
                ids: named.iter().map(|list| clean(&list.id)).collect(),
            }
            .into());
        }
    };
    if list.writable {
        Ok(list)
    } else {
        Err(SelectError::ReadOnlyList {
            name: clean(&list.name),
        }
        .into())
    }
}

fn expected(before: &StoredReminder, edit: &ReminderEdit) -> StoredReminder {
    let mut reminder = before.clone();
    if let Some(completed) = edit.completed {
        reminder.completed = completed;
    }
    match &edit.priority {
        FieldEdit::Set(priority) => reminder.priority = Some(*priority),
        FieldEdit::Remove => reminder.priority = None,
        FieldEdit::Unchanged => {}
    }
    if let Some(list_id) = &edit.list_id {
        reminder.list_id.clone_from(list_id);
    }
    reminder
}

fn list_name(lists: &[ReminderList], id: &str) -> String {
    lists
        .iter()
        .find(|list| list.id == id)
        .map_or_else(|| id.to_owned(), |list| list.name.clone())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::app::undo;
    use crate::domain::{
        ApplyResult, Calendar, JournalAction, NewReminder, Notes, Priority, Reminder,
        ReminderSnapshot, Reminders,
    };
    use crate::infra::host::Scripted;

    type Shared = Rc<RefCell<Vec<StoredReminder>>>;
    type Calls = Rc<RefCell<Vec<String>>>;

    struct Store {
        reminders: Shared,
        calls: Calls,
    }

    impl Reminders for Store {
        fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
            unreachable!()
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            unreachable!()
        }

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            self.calls.borrow_mut().push("snapshot".into());
            Ok(ReminderSnapshot {
                lists: lists(),
                reminders: self
                    .reminders
                    .borrow()
                    .iter()
                    .filter(|reminder| !reminder.completed)
                    .cloned()
                    .collect(),
            })
        }

        fn snapshot_including_completed(&self) -> Result<ReminderSnapshot, BackendError> {
            self.calls
                .borrow_mut()
                .push("snapshot_including_completed".into());
            Ok(ReminderSnapshot {
                lists: lists(),
                reminders: self.reminders.borrow().clone(),
            })
        }

        fn reminder(&self, id: &str) -> Result<Option<StoredReminder>, BackendError> {
            self.calls.borrow_mut().push(format!("reminder {id}"));
            Ok(self
                .reminders
                .borrow()
                .iter()
                .find(|reminder| reminder.id == id)
                .cloned())
        }

        fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
            let mut reminders = self.reminders.borrow_mut();
            let mut written = Vec::new();
            for change in changes {
                let ReminderChange::Update { before, edit } = change else {
                    unreachable!("one-step commands only update reminders")
                };
                let current = reminders
                    .iter_mut()
                    .find(|reminder| reminder.id == before.id)
                    .unwrap();
                assert_eq!(current, before, "apply must see the selected state");
                *current = expected(before, edit);
                written.push(Some(current.clone()));
            }
            Ok(ApplyResult {
                unrecorded: vec![false; written.len()],
                written,
            })
        }
    }

    fn lists() -> Vec<ReminderList> {
        vec![
            ReminderList {
                id: "list-home".into(),
                name: "Home".into(),
                writable: true,
            },
            ReminderList {
                id: "list-work".into(),
                name: "Work".into(),
                writable: true,
            },
            ReminderList {
                id: "list-shared".into(),
                name: "Shared".into(),
                writable: false,
            },
        ]
    }

    fn reminder(id: &str, title: &str, list_id: &str, completed: bool) -> StoredReminder {
        StoredReminder {
            id: id.into(),
            list_id: list_id.into(),
            title: title.into(),
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

    fn sample() -> Vec<StoredReminder> {
        let mut report = reminder("rem-3", "File report", "list-work", false);
        report.priority = Some(Priority::High);
        vec![
            reminder("rem-1", "Buy oat milk", "list-home", false),
            reminder("rem-2", "Buy eggs", "list-work", false),
            report,
            reminder("rem-4", "Call the dentist", "list-home", true),
            reminder("rem-5", "Buy stamps", "list-home", true),
        ]
    }

    fn now() -> Zoned {
        date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn backends(reminders: &Shared, calls: &Calls) -> Backends {
        let reminders = reminders.clone();
        let calls = calls.clone();
        Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(Store { reminders, calls }),
            || -> Box<dyn Calendar> { unreachable!() },
        )
    }

    fn request(change: &TaskChange, selectors: &[&str]) -> TaskChangeRequest {
        TaskChangeRequest {
            change: change.clone(),
            selectors: selectors.iter().map(ToString::to_string).collect(),
            list: None,
        }
    }

    fn change(
        change: &TaskChange,
        selectors: &[&str],
    ) -> (Result<Vec<Changed>>, Vec<StoredReminder>, Scripted) {
        let (result, state, host, _) = change_with_calls(change, selectors);
        (result, state, host)
    }

    fn change_with_calls(
        change: &TaskChange,
        selectors: &[&str],
    ) -> (
        Result<Vec<Changed>>,
        Vec<StoredReminder>,
        Scripted,
        Vec<String>,
    ) {
        let reminders = Shared::new(RefCell::new(sample()));
        let calls = Calls::default();
        let mut host = Scripted::new();
        let result = run(
            &request(change, selectors),
            &now(),
            &backends(&reminders, &calls),
            &mut host,
        );
        let state = reminders.borrow().clone();
        let calls = calls.borrow().clone();
        (result, state, host, calls)
    }

    fn select_ids(
        task_change: &TaskChange,
        selectors: &[&str],
        list: Option<&str>,
    ) -> Result<Vec<String>> {
        let all = sample();
        let selectors: Vec<String> = selectors.iter().map(ToString::to_string).collect();
        select(task_change, &selectors, list, &lists(), &all, |_: &str| {
            Ok(None)
        })
        .map(|selected| selected.into_iter().map(|reminder| reminder.id).collect())
    }

    #[test]
    fn selectors_found_in_the_snapshot_skip_the_id_fetch() {
        let all = sample();
        let open: Vec<StoredReminder> = all
            .iter()
            .filter(|reminder| !reminder.completed)
            .cloned()
            .collect();
        let mut fetched = Vec::new();
        let selectors: Vec<String> = ["rem-2", "OAT", "report", "rem-4"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let selected = select(
            &TaskChange::Priority(None),
            &selectors,
            None,
            &lists(),
            &open,
            |id: &str| {
                fetched.push(id.to_owned());
                Ok(all.iter().find(|reminder| reminder.id == id).cloned())
            },
        )
        .unwrap();
        assert_eq!(
            selected
                .into_iter()
                .map(|reminder| reminder.id)
                .collect::<Vec<_>>(),
            ["rem-2", "rem-1", "rem-3", "rem-4"]
        );
        assert_eq!(fetched, ["rem-4"]);
    }

    #[test]
    fn selectors_accept_ids_and_unique_case_insensitive_title_parts() {
        assert_eq!(
            select_ids(&TaskChange::Complete, &["rem-2", "OAT"], None).unwrap(),
            ["rem-2", "rem-1"]
        );
        assert_eq!(
            select_ids(&TaskChange::Reopen, &["dentist"], None).unwrap(),
            ["rem-4"]
        );
    }

    #[test]
    fn the_same_reminder_selected_twice_is_changed_once() {
        assert_eq!(
            select_ids(&TaskChange::Complete, &["oat", "rem-1", "OAT MILK"], None).unwrap(),
            ["rem-1"]
        );
    }

    #[test]
    fn an_ambiguous_title_lists_every_candidate_with_its_id_and_list() {
        let error = select_ids(&TaskChange::Complete, &["buy"], None).unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"buy\" matches 2 open reminders:\n  rem-1  \"Buy oat milk\"  (Home)\n  rem-2  \"Buy eggs\"  (Work)"
        );
    }

    #[test]
    fn list_narrows_title_matching_but_not_ids() {
        assert_eq!(
            select_ids(&TaskChange::Complete, &["buy"], Some("Work")).unwrap(),
            ["rem-2"]
        );
        assert_eq!(
            select_ids(&TaskChange::Complete, &["rem-1"], Some("Work")).unwrap(),
            ["rem-1"]
        );
        assert_eq!(
            select_ids(&TaskChange::Complete, &["oat"], Some("Work"))
                .unwrap_err()
                .to_string(),
            "no open reminder matches \"oat\" in \"Work\""
        );
        assert_eq!(
            select_ids(&TaskChange::Complete, &["oat"], Some("Nope"))
                .unwrap_err()
                .to_string(),
            "reminder list \"Nope\" not found"
        );
    }

    #[test]
    fn missing_titles_name_the_state_that_was_searched() {
        assert_eq!(
            select_ids(&TaskChange::Complete, &["dentist"], None)
                .unwrap_err()
                .to_string(),
            "no open reminder matches \"dentist\""
        );
        assert_eq!(
            select_ids(&TaskChange::Reopen, &["oat"], None)
                .unwrap_err()
                .to_string(),
            "no completed reminder matches \"oat\""
        );
    }

    #[test]
    fn ids_in_the_wrong_state_are_errors_for_done_and_reopen() {
        assert_eq!(
            select_ids(&TaskChange::Complete, &["rem-4"], None)
                .unwrap_err()
                .to_string(),
            "\"Call the dentist\" is already completed"
        );
        assert_eq!(
            select_ids(&TaskChange::Reopen, &["rem-1"], None)
                .unwrap_err()
                .to_string(),
            "\"Buy oat milk\" isn't completed"
        );
        assert_eq!(
            select_ids(&TaskChange::Priority(None), &["rem-4"], None).unwrap(),
            ["rem-4"]
        );
    }

    #[test]
    fn a_bad_selector_changes_nothing() {
        let (result, state, host) = change(&TaskChange::Complete, &["oat", "nothing"]);
        assert!(result.unwrap_err().to_string().contains("\"nothing\""));
        assert_eq!(state, sample());
        assert_eq!(host.journals, Vec::<(String, String)>::new());
        assert_eq!(host.backups, Vec::<(String, String)>::new());
    }

    #[test]
    fn done_completes_and_journals_the_change() {
        let (result, state, host) = change(&TaskChange::Complete, &["oat", "eggs"]);
        let changed = result.unwrap();
        assert_eq!(changed.len(), 2);
        assert!(changed.iter().all(|item| item.reminder.completed));
        assert_eq!(changed[0].list, "Home");
        assert!(state[0].completed && state[1].completed);
        assert_eq!(host.backups.len(), 1);
        assert_eq!(host.backups[0].0, "20261026T090000-tasks.json");
        assert_eq!(host.journals.len(), 1);
        let journal: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        assert_eq!(journal["action"], "task_done");
        assert_eq!(journal["changes"][0]["change"], "completed");
        assert_eq!(host.said, Vec::<String>::new());
    }

    #[test]
    fn reopen_marks_completed_reminders_open() {
        let (result, state, host) = change(&TaskChange::Reopen, &["dentist"]);
        let changed = result.unwrap();
        assert_eq!(changed[0].reminder.id, "rem-4");
        assert!(!state[3].completed);
        let journal: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        assert_eq!(journal["action"], "task_reopen");
        assert_eq!(journal["changes"][0]["change"], "updated");
    }

    #[test]
    fn reopen_reads_completed_reminders_from_one_full_snapshot() {
        let (result, state, _, calls) = change_with_calls(&TaskChange::Reopen, &["dentist"]);
        assert_eq!(result.unwrap()[0].reminder.id, "rem-4");
        assert!(!state[3].completed);
        assert_eq!(calls, ["snapshot_including_completed"]);

        let (result, state, _, calls) =
            change_with_calls(&TaskChange::Reopen, &["rem-5", "nothing"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "no completed reminder matches \"nothing\""
        );
        assert_eq!(state, sample());
        assert_eq!(calls, ["snapshot_including_completed"]);
    }

    #[test]
    fn reopen_by_id_finds_completed_reminders_and_refuses_open_ones() {
        let (result, state, _, calls) = change_with_calls(&TaskChange::Reopen, &["rem-5"]);
        assert_eq!(result.unwrap()[0].reminder.title, "Buy stamps");
        assert!(!state[4].completed);
        assert_eq!(calls, ["snapshot_including_completed"]);

        let (result, _, _, _) = change_with_calls(&TaskChange::Reopen, &["rem-1"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "\"Buy oat milk\" isn't completed"
        );
    }

    #[test]
    fn done_priority_and_move_match_titles_among_open_reminders_only() {
        for task_change in [
            TaskChange::Complete,
            TaskChange::Priority(Some(Priority::Low)),
            TaskChange::Move("Work".into()),
        ] {
            let (result, state, _, calls) = change_with_calls(&task_change, &["stamps"]);
            assert_eq!(
                result.unwrap_err().to_string(),
                "no open reminder matches \"stamps\"",
                "{task_change:?}"
            );
            assert_eq!(state, sample());
            assert_eq!(calls, ["snapshot", "reminder stamps"], "{task_change:?}");
        }
    }

    #[test]
    fn done_on_a_completed_id_fetches_it_to_report_its_state() {
        let (result, _, _, calls) = change_with_calls(&TaskChange::Complete, &["rem-4"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "\"Call the dentist\" is already completed"
        );
        assert_eq!(calls, ["snapshot", "reminder rem-4"]);
    }

    #[test]
    fn priority_sets_clears_and_skips_reminders_that_already_match() {
        let (result, state, host) = change(&TaskChange::Priority(Some(Priority::High)), &["oat"]);
        assert_eq!(result.unwrap()[0].reminder.priority, Some(Priority::High));
        assert_eq!(state[0].priority, Some(Priority::High));
        let journal: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        assert_eq!(journal["action"], "task_priority");

        let (result, state, _) = change(&TaskChange::Priority(None), &["report"]);
        assert_eq!(result.unwrap()[0].reminder.priority, None);
        assert_eq!(state[2].priority, None);

        let (result, state, host) =
            change(&TaskChange::Priority(Some(Priority::High)), &["report"]);
        assert_eq!(result.unwrap(), Vec::new());
        assert_eq!(state, sample());
        assert_eq!(host.journals, Vec::<(String, String)>::new());
    }

    #[test]
    fn move_changes_the_list_and_reports_its_name() {
        let (result, state, host) = change(&TaskChange::Move("Work".into()), &["oat"]);
        let changed = result.unwrap();
        assert_eq!(changed[0].list, "Work");
        assert_eq!(state[0].list_id, "list-work");
        let journal: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        assert_eq!(journal["action"], "task_move");

        let (result, _, _) = change(&TaskChange::Move("list-home".into()), &["eggs"]);
        assert_eq!(result.unwrap()[0].list, "Home");

        let (result, state, _) = change(&TaskChange::Move("Work".into()), &["eggs"]);
        assert_eq!(result.unwrap(), Vec::new());
        assert_eq!(state, sample());
    }

    #[test]
    fn move_rejects_missing_read_only_and_ambiguous_lists() {
        let (result, state, _) = change(&TaskChange::Move("Nope".into()), &["oat"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "reminder list \"Nope\" not found"
        );
        assert_eq!(state, sample());

        let (result, _, _) = change(&TaskChange::Move("Shared".into()), &["oat"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "reminder list \"Shared\" is read-only"
        );

        let mut duplicated = lists();
        duplicated[1].name = "Home".into();
        let error = plan(
            &TaskChange::Move("Home".into()),
            &[reminder("rem-2", "Buy eggs", "list-work", false)],
            &duplicated,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "more than one reminder list is named \"Home\" (list-home, list-work)"
        );
    }

    #[test]
    fn undo_reverses_a_done() {
        let reminders = Shared::new(RefCell::new(sample()));
        let backends = backends(&reminders, &Calls::default());
        let mut host = Scripted::new().answer("y");
        run(
            &request(&TaskChange::Complete, &["oat"]),
            &now(),
            &backends,
            &mut host,
        )
        .unwrap();
        assert!(reminders.borrow()[0].completed);

        let log = journal::log(now().timestamp(), &mut host).unwrap();
        assert_eq!(log[0].action, JournalAction::TaskDone);
        assert_eq!(log[0].counts.completed, 1);

        let outcome = undo::run(None, &now(), &backends, &mut host).unwrap();
        assert!(matches!(outcome, undo::Outcome::Applied { .. }));
        assert!(
            host.said
                .iter()
                .any(|line| line.contains("○ incomplete \"Buy oat milk\""))
        );
        assert_eq!(*reminders.borrow(), sample());
    }

    fn assert_undo_restores(task_change: &TaskChange, selector: &str, action: JournalAction) {
        let reminders = Shared::new(RefCell::new(sample()));
        let backends = backends(&reminders, &Calls::default());
        let mut host = Scripted::new().answer("y");
        let changed = run(
            &request(task_change, &[selector]),
            &now(),
            &backends,
            &mut host,
        )
        .unwrap();
        assert_eq!(changed.len(), 1);
        assert_ne!(*reminders.borrow(), sample());

        let log = journal::log(now().timestamp(), &mut host).unwrap();
        assert_eq!(log[0].action, action);

        let outcome = undo::run(None, &now(), &backends, &mut host).unwrap();
        assert!(matches!(outcome, undo::Outcome::Applied { .. }));
        assert_eq!(*reminders.borrow(), sample());
    }

    #[test]
    fn undo_reverses_a_reopen() {
        assert_undo_restores(&TaskChange::Reopen, "dentist", JournalAction::TaskReopen);
    }

    #[test]
    fn undo_reverses_setting_changing_and_clearing_a_priority() {
        assert_undo_restores(
            &TaskChange::Priority(Some(Priority::Medium)),
            "oat",
            JournalAction::TaskPriority,
        );
        assert_undo_restores(
            &TaskChange::Priority(Some(Priority::Low)),
            "report",
            JournalAction::TaskPriority,
        );
        assert_undo_restores(
            &TaskChange::Priority(None),
            "report",
            JournalAction::TaskPriority,
        );
    }

    #[test]
    fn undo_reverses_a_move() {
        assert_undo_restores(
            &TaskChange::Move("Work".into()),
            "oat",
            JournalAction::TaskMove,
        );
    }
}

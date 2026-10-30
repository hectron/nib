use std::path::PathBuf;
use std::time::Duration;

use jiff::Zoned;
use miette::{IntoDiagnostic, Result, miette};
use serde_json::json;

use crate::app::journal;
use crate::app::markdown::{self, ParseError, Plan, Session};
use crate::cli::output::{clean, clean_lines};
use crate::domain::{Backends, ReminderChange};
use crate::infra::host::Host;

const INSTANT: Duration = Duration::from_millis(500);

pub fn tasks(
    list: Option<&str>,
    now: &Zoned,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<String> {
    let snapshot = backends.reminders.snapshot(list)?;
    let session = Session::new_at(&snapshot, now);
    if session.skipped > 0 {
        host.say(&format!(
            "Skipping {} with line breaks or control characters in the text; nib won't show or change {}.",
            count(session.skipped, "reminder"),
            if session.skipped == 1 { "it" } else { "them" }
        ));
    }
    let original = markdown::render(&session.document);
    let mut file = host.create_edit_file(&original).into_diagnostic()?;
    loop {
        let elapsed = host.edit(file.path()).into_diagnostic()?;
        file.preserve();
        let edited = file.read().into_diagnostic()?;
        if edited == original {
            if elapsed < INSTANT {
                host.say("Your editor returned right away. If it opens a window, make it wait, for example EDITOR=\"code --wait\".");
            }
            file.remove_on_drop();
            return Ok("No changes.".into());
        }
        let plan = match markdown::parse(&edited)
            .and_then(|document| markdown::plan(&snapshot, &session, &document))
            .and_then(|plan| {
                validate_time_zones(&plan, backends)?;
                Ok(plan)
            }) {
            Ok(plan) => plan,
            Err(e) => {
                host.say(&clean(&e.to_string()));
                if edit_again(host)? {
                    continue;
                }
                return Err(miette!(
                    "Nothing was changed. Your edits are in {}",
                    file.path().display()
                ));
            }
        };
        if plan.changes.is_empty() {
            for warning in &plan.warnings {
                host.say(&clean(warning));
            }
            file.remove_on_drop();
            return Ok("No changes.".into());
        }
        host.say(&clean_lines(&markdown::describe(&plan, &snapshot)));
        if !confirmed(host, &plan)? {
            return Ok(format!(
                "Nothing was changed. Your edits are in {}",
                file.path().display()
            ));
        }
        let _lock = host.lock_journal().into_diagnostic()?;
        if let Some(saved) = back_up(now, &plan.changes, host)? {
            host.say(&format!(
                "Backed up the reminders being changed to {}",
                saved.display()
            ));
        }
        let applied = match backends.reminders.apply(&plan.changes) {
            Ok(applied) => applied,
            Err(e) => {
                host.say(&format!(
                    "Your edits are still in {}",
                    file.path().display()
                ));
                return Err(e.into());
            }
        };
        if let Err(error) = journal::record_edit(now, &plan.changes, &applied, host) {
            host.say("The changes were applied, but nib couldn't record the journal entry.");
            file.remove_on_drop();
            return Err(error);
        }
        file.remove_on_drop();
        return Ok(summary(&plan));
    }
}

fn validate_time_zones(plan: &Plan, backends: &Backends) -> std::result::Result<(), ParseError> {
    for (line, zone) in &plan.time_zones {
        backends
            .reminders
            .validate_time_zone(zone)
            .map_err(|error| ParseError {
                line: *line,
                message: error.to_string(),
            })?;
    }
    Ok(())
}

fn edit_again(host: &mut dyn Host) -> Result<bool> {
    let answer = host.ask("Edit again? [Y/n] ").into_diagnostic()?;
    Ok(answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn confirmed(host: &mut dyn Host, plan: &Plan) -> Result<bool> {
    let deletions = plan.deletions();
    let backup_warning = backup_warning(plan);
    if deletions > 3 {
        let answer = host
            .ask(&format!(
                "This deletes {deletions} reminders.{backup_warning} Type {deletions} to apply: "
            ))
            .into_diagnostic()?;
        return Ok(answer == deletions.to_string());
    }
    let answer = host
        .ask(&format!(
            "Apply {}?{backup_warning} [y/N] ",
            count(plan.changes.len(), "change"),
        ))
        .into_diagnostic()?;
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn backup_warning(plan: &Plan) -> String {
    let partial = plan
        .changes
        .iter()
        .filter_map(|change| match change {
            ReminderChange::Add { .. } => None,
            ReminderChange::Restore { before } => before.backup_partial.then_some(()),
            ReminderChange::Update { before, .. } | ReminderChange::Delete { before } => {
                before.backup_partial.then_some(())
            }
        })
        .count();
    if partial == 0 {
        String::new()
    } else {
        format!(
            " The backup will be partial for {} because EventKit returned data nib couldn't capture.",
            count(partial, "reminder")
        )
    }
}

pub(crate) fn back_up(
    now: &Zoned,
    changes: &[ReminderChange],
    host: &mut dyn Host,
) -> Result<Option<PathBuf>> {
    let Some(contents) = backup_changes(changes)? else {
        return Ok(None);
    };
    let name = format!("{}-tasks.json", now.strftime("%Y%m%dT%H%M%S"));
    host.save_backup(&name, &contents)
        .map(Some)
        .into_diagnostic()
}

pub(crate) fn backup_changes(changes: &[ReminderChange]) -> Result<Option<String>> {
    let items: Vec<_> = changes
        .iter()
        .filter_map(|change| match change {
            ReminderChange::Add { .. } | ReminderChange::Restore { .. } => None,
            ReminderChange::Update { before, .. } => Some(("update", before)),
            ReminderChange::Delete { before } => Some(("delete", before)),
        })
        .map(|(change, before)| {
            let due_components = before.due_components.as_ref().map(|due| {
                json!({
                    "calendar": due.calendar,
                    "time_zone": due.time_zone,
                    "era": due.era,
                    "year": due.year,
                    "month": due.month,
                    "day": due.day,
                    "hour": due.hour,
                    "minute": due.minute,
                    "second": due.second,
                    "nanosecond": due.nanosecond,
                    "weekday": due.weekday,
                    "weekday_ordinal": due.weekday_ordinal,
                    "quarter": due.quarter,
                    "week_of_month": due.week_of_month,
                    "week_of_year": due.week_of_year,
                    "year_for_week_of_year": due.year_for_week_of_year,
                    "day_of_year": due.day_of_year,
                    "is_leap_month": due.is_leap_month,
                })
            });
            json!({
                "change": change,
                "id": before.id,
                "list_id": before.list_id,
                "title": before.title,
                "notes": before.notes,
                "due": before.due.map(|d| d.to_string()),
                "due_components": due_components,
                "priority": before.priority.map(crate::domain::Priority::as_str),
                "completed": before.completed,
                "modified": before.modified.map(|t| t.to_string()),
                "alarms_present": before.alarms_present,
                "absolute_alarms": before.absolute_alarms,
                "recurrence_present": before.recurrence_present,
                "url": before.url,
                "partial": before.backup_partial,
            })
        })
        .collect();
    if items.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string_pretty(&items)
            .map(Some)
            .into_diagnostic()
    }
}

fn summary(plan: &Plan) -> String {
    let (mut added, mut updated, mut completed, mut deleted) = (0, 0, 0, 0);
    for change in &plan.changes {
        match change {
            ReminderChange::Add { .. } | ReminderChange::Restore { .. } => added += 1,
            ReminderChange::Update { edit, .. } if edit.completed == Some(true) => completed += 1,
            ReminderChange::Update { .. } => updated += 1,
            ReminderChange::Delete { .. } => deleted += 1,
        }
    }
    let parts: Vec<String> = [
        (added, "added"),
        (updated, "updated"),
        (completed, "completed"),
        (deleted, "deleted"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    format!(
        "Applied {}: {}.",
        count(plan.changes.len(), "change"),
        parts.join(", ")
    )
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
    use std::fs;
    use std::rc::Rc;

    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::domain::{
        ApplyResult, BackendError, Calendar, FieldEdit, NewReminder, Notes, Reminder,
        ReminderDueComponents, ReminderList, ReminderSnapshot, Reminders, StoredReminder,
    };
    use crate::infra::host::Scripted;

    type Applied = Rc<RefCell<Vec<ReminderChange>>>;

    struct Recorder {
        snapshot: ReminderSnapshot,
        applied: Applied,
        conflict: bool,
        unrecorded: bool,
        journal_locks: Rc<Cell<usize>>,
    }

    impl Reminders for Recorder {
        fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
            unreachable!()
        }

        fn list(&self) -> Result<Vec<Reminder>, BackendError> {
            unreachable!()
        }

        fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
            Ok(self.snapshot.clone())
        }

        fn apply(&self, changes: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
            assert!(
                self.journal_locks.get() > 0,
                "apply requires the journal lock"
            );
            if self.conflict {
                return Err(BackendError::Conflict {
                    titles: vec!["\"Oat milk\"".into()],
                });
            }
            self.applied.borrow_mut().extend_from_slice(changes);
            Ok(ApplyResult {
                written: changes.iter().map(fake_written).collect(),
                unrecorded: vec![self.unrecorded; changes.len()],
            })
        }
    }

    fn stored(id: &str, title: &str) -> StoredReminder {
        StoredReminder {
            id: id.into(),
            list_id: "list-1".into(),
            title: title.into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            completed: false,
            modified: None,
            alarms_present: false,
            absolute_alarms: Some(Vec::new()),
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    fn fake_written(change: &ReminderChange) -> Option<StoredReminder> {
        match change {
            ReminderChange::Add {
                list_id,
                title,
                notes,
                due,
                due_time,
                due_time_zone,
                priority,
            } => Some(StoredReminder {
                id: format!("added-{title}"),
                list_id: list_id.clone(),
                title: title.clone(),
                notes: notes.clone(),
                due: *due,
                due_components: due.map(|date| crate::domain::ReminderDueComponents {
                    year: Some(date.year().into()),
                    month: Some(date.month().into()),
                    day: Some(date.day().into()),
                    hour: due_time.map(|time| i64::from(time.hour())),
                    minute: due_time.map(|time| i64::from(time.minute())),
                    time_zone: due_time_zone.clone(),
                    ..crate::domain::ReminderDueComponents::default()
                }),
                priority: *priority,
                completed: false,
                modified: None,
                alarms_present: false,
                absolute_alarms: Some(Vec::new()),
                recurrence_present: false,
                url: None,
                backup_partial: false,
            }),
            ReminderChange::Restore { before } => {
                let mut written = before.clone();
                written.id = format!("restored-{}", before.id);
                Some(written)
            }
            ReminderChange::Update { before, edit } => {
                let mut written = before.clone();
                if let Some(title) = &edit.title {
                    written.title.clone_from(title);
                }
                apply_field(&mut written.notes, &edit.notes);
                apply_field(&mut written.due, &edit.due);
                apply_field(&mut written.priority, &edit.priority);
                if let Some(list_id) = &edit.list_id {
                    written.list_id.clone_from(list_id);
                }
                if let Some(completed) = edit.completed {
                    written.completed = completed;
                }
                Some(written)
            }
            ReminderChange::Delete { .. } => None,
        }
    }

    fn apply_field<T: Clone>(value: &mut Option<T>, edit: &FieldEdit<T>) {
        match edit {
            FieldEdit::Unchanged => {}
            FieldEdit::Set(new) => *value = Some(new.clone()),
            FieldEdit::Remove => *value = None,
        }
    }

    fn snapshot(titles: &[&str]) -> ReminderSnapshot {
        ReminderSnapshot {
            lists: vec![ReminderList {
                id: "list-1".into(),
                name: "Reminders".into(),
                writable: true,
            }],
            reminders: titles
                .iter()
                .enumerate()
                .map(|(i, title)| stored(&format!("rem-{i:04}"), title))
                .collect(),
        }
    }

    fn run(
        titles: &[&str],
        conflict: bool,
        host: &mut Scripted,
    ) -> (Result<String>, Vec<ReminderChange>) {
        run_snapshot(snapshot(titles), conflict, host)
    }

    fn run_snapshot(
        snapshot: ReminderSnapshot,
        conflict: bool,
        host: &mut Scripted,
    ) -> (Result<String>, Vec<ReminderChange>) {
        run_snapshot_with_recording(snapshot, conflict, false, host)
    }

    fn run_snapshot_with_recording(
        snapshot: ReminderSnapshot,
        conflict: bool,
        unrecorded: bool,
        host: &mut Scripted,
    ) -> (Result<String>, Vec<ReminderChange>) {
        let applied = Applied::default();
        let journal_locks = host.journal_lock_state();
        let recorder = Recorder {
            snapshot,
            applied: applied.clone(),
            conflict,
            unrecorded,
            journal_locks,
        };
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(recorder),
            || -> Box<dyn Calendar> { unreachable!() },
        );
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let result = tasks(None, &now, &backends, host);
        let applied = applied.borrow().clone();
        (result, applied)
    }

    fn check(line: &str) -> impl FnOnce(String) -> String {
        let line = line.to_string();
        move |text| text.replace(&format!("- [ ] {line}"), &format!("- [x] {line}"))
    }

    #[test]
    fn confirmed_edits_are_backed_up_then_applied() {
        let mut host = Scripted::new().edit(check("Oat milk")).answer("y");
        let (result, applied) = run(&["Oat milk", "Eggs"], false, &mut host);
        assert_eq!(result.unwrap(), "Applied 1 change: 1 completed.");
        assert!(
            matches!(&applied[..], [ReminderChange::Update { edit, .. }] if edit.completed == Some(true))
        );
        assert_eq!(host.backups.len(), 1);
        assert_eq!(host.backups[0].0, "20261026T090000-tasks.json");
        assert!(host.backups[0].1.contains("\"Oat milk\""));
        assert_eq!(host.journals.len(), 1);
        assert_eq!(host.journals[0].0, "20261026T090000-journal.json");
        assert!(host.journals[0].1.contains("\"change\": \"completed\""));
        assert!(
            host.said
                .iter()
                .any(|s| s.contains("✓ complete  \"Oat milk\""))
        );
    }

    #[test]
    fn post_commit_recording_problem_still_writes_the_edit_journal() {
        let mut host = Scripted::new().edit(check("Oat milk")).answer("y");

        let (result, applied) =
            run_snapshot_with_recording(snapshot(&["Oat milk"]), false, true, &mut host);

        assert_eq!(result.unwrap(), "Applied 1 change: 1 completed.");
        assert_eq!(applied.len(), 1);
        assert_eq!(host.journals.len(), 1);
        assert!(host.journals[0].1.contains("\"unrecorded\": true"));
        assert!(host.said.iter().any(|line| line
            == "warning: nib couldn't re-read 1 reminder after saving; undo of this entry is limited"));
        assert!(!host.said.iter().any(|line| line.contains("still in")));
    }

    #[test]
    fn backups_include_native_metadata_and_disclose_partial_data() {
        let mut snapshot = snapshot(&["Oat milk"]);
        snapshot.reminders[0].due = Some(date(2026, 10, 27));
        snapshot.reminders[0].due_components = Some(ReminderDueComponents {
            calendar: Some("gregorian".into()),
            time_zone: Some("America/Chicago".into()),
            era: Some(1),
            year: Some(2026),
            month: Some(10),
            day: Some(27),
            hour: Some(14),
            minute: Some(30),
            second: Some(15),
            nanosecond: Some(0),
            weekday: Some(3),
            weekday_ordinal: Some(4),
            quarter: Some(4),
            week_of_month: Some(5),
            week_of_year: Some(44),
            year_for_week_of_year: Some(2026),
            day_of_year: Some(300),
            is_leap_month: false,
        });
        snapshot.reminders[0].alarms_present = true;
        snapshot.reminders[0].recurrence_present = true;
        snapshot.reminders[0].url = Some("https://example.com/reminder".into());
        snapshot.reminders[0].backup_partial = true;
        let mut host = Scripted::new().edit(check("Oat milk")).answer("y");
        let (result, _) = run_snapshot(snapshot, false, &mut host);
        assert!(result.is_ok());
        let backup: serde_json::Value = serde_json::from_str(&host.backups[0].1).unwrap();
        assert_eq!(backup[0]["alarms_present"], true);
        assert_eq!(backup[0]["recurrence_present"], true);
        assert_eq!(backup[0]["url"], "https://example.com/reminder");
        assert_eq!(backup[0]["due_components"]["hour"], 14);
        assert_eq!(backup[0]["due_components"]["time_zone"], "America/Chicago");
        assert!(
            host.said
                .iter()
                .any(|message| message.contains("backup will be partial"))
        );
    }

    #[test]
    fn saving_without_changes_does_nothing() {
        let mut host = Scripted::new().edit(|text| text);
        let (result, applied) = run(&["Oat milk"], false, &mut host);
        assert_eq!(result.unwrap(), "No changes.");
        assert_eq!(applied, Vec::<ReminderChange>::new());
        assert_eq!(host.backups, Vec::<(String, String)>::new());
    }

    #[test]
    fn unavailable_zones_use_the_line_numbered_re_edit_flow() {
        struct RejectZone(ReminderSnapshot);

        impl Reminders for RejectZone {
            fn add(&self, _: NewReminder) -> Result<Reminder, BackendError> {
                unreachable!()
            }

            fn list(&self) -> Result<Vec<Reminder>, BackendError> {
                unreachable!()
            }

            fn snapshot(&self, _: Option<&str>) -> Result<ReminderSnapshot, BackendError> {
                Ok(self.0.clone())
            }

            fn apply(&self, _: &[ReminderChange]) -> Result<ApplyResult, BackendError> {
                panic!("validation must happen before apply")
            }

            fn validate_time_zone(&self, name: &str) -> Result<(), BackendError> {
                Err(BackendError::Failed {
                    app: "Reminders",
                    message: format!("time zone {name:?} isn't available"),
                })
            }
        }

        let mut dated = stored("rem-0000", "File report");
        dated.due = Some(date(2026, 10, 27));
        dated.due_components = Some(ReminderDueComponents {
            year: Some(2026),
            month: Some(10),
            day: Some(27),
            ..ReminderDueComponents::default()
        });
        let snapshot = ReminderSnapshot {
            lists: snapshot(&[]).lists,
            reminders: vec![dated],
        };
        let retry_snapshot = snapshot.clone();
        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(RejectZone(snapshot)),
            || -> Box<dyn Calendar> { unreachable!() },
        );
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let mut host = Scripted::new()
            .edit(|text| text.replace("2026-10-27", "2026-10-27 09:00"))
            .answer("n");
        let error = tasks(None, &now, &backends, &mut host).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Nothing was changed. Your edits are in")
        );
        assert!(
            host.said
                .iter()
                .any(|line| line.starts_with("line ") && line.contains("isn't available"))
        );
        assert!(host.said.iter().any(|line| line == "Edit again? [Y/n] "));
        assert!(!host.said.iter().any(|line| line.contains("~ update")));

        let backends = Backends::new(
            || -> Box<dyn Notes> { unreachable!() },
            move || Box::new(RejectZone(retry_snapshot)),
            || -> Box<dyn Calendar> { unreachable!() },
        );
        let mut host = Scripted::new()
            .edit(|text| text.replace("2026-10-27", "2026-10-27 09:00"))
            .answer("")
            .edit(|text| {
                assert!(text.contains("2026-10-27 09:00"));
                text.replace("2026-10-27 09:00", "2026-10-27")
            });
        assert_eq!(
            tasks(None, &now, &backends, &mut host).unwrap(),
            "No changes."
        );
    }

    #[test]
    fn a_parse_error_reopens_the_editor_with_the_edits() {
        let mut host = Scripted::new()
            .edit(|text| text.replace("- [ ] Oat milk", "- [ ] Oat milk\nstray text"))
            .answer("")
            .edit(|text| {
                assert!(text.contains("stray text"), "the edits were lost");
                text.replace("\nstray text", "")
                    .replace("- [ ] Oat milk", "- [x] Oat milk")
            })
            .answer("y");
        let (result, applied) = run(&["Oat milk"], false, &mut host);
        assert_eq!(result.unwrap(), "Applied 1 change: 1 completed.");
        assert_eq!(applied.len(), 1);
        assert!(
            host.said
                .iter()
                .any(|s| s.starts_with("line ") && s.contains("expected a reminder"))
        );
    }

    #[test]
    fn giving_up_after_an_error_changes_nothing() {
        let mut host = Scripted::new()
            .edit(|text| text.replace("- [ ] Oat milk", "- [ ] Oat milk\nstray text"))
            .answer("n");
        let (result, applied) = run(&["Oat milk"], false, &mut host);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("Nothing was changed. Your edits are in")
        );
        assert_eq!(applied, Vec::<ReminderChange>::new());
    }

    #[test]
    fn declining_keeps_the_edits_and_changes_nothing() {
        let mut host = Scripted::new().edit(check("Oat milk")).answer("n");
        let (result, applied) = run(&["Oat milk"], false, &mut host);
        let message = result.unwrap();
        assert!(
            message.starts_with("Nothing was changed. Your edits are in"),
            "{message}"
        );
        let path = message.rsplit(' ').next().unwrap();
        assert!(fs::read_to_string(path).unwrap().contains("- [x] Oat milk"));
        fs::remove_file(path).unwrap();
        assert_eq!(applied, Vec::<ReminderChange>::new());
        assert_eq!(host.backups, Vec::<(String, String)>::new());
    }

    #[test]
    fn deleting_more_than_three_needs_the_count_typed() {
        let titles = ["Keep", "One", "Two", "Three", "Four"];
        let delete_four = |text: String| {
            text.lines()
                .filter(|l| {
                    !["One", "Two", "Three", "Four"]
                        .iter()
                        .any(|t| l.contains(&format!("] {t} ")))
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let mut host = Scripted::new().edit(delete_four).answer("y");
        let (result, applied) = run(&titles, false, &mut host);
        assert!(result.unwrap().starts_with("Nothing was changed"));
        assert_eq!(applied, Vec::<ReminderChange>::new());
        assert!(host.said.iter().any(|s| s.contains("Type 4 to apply")));

        let mut host = Scripted::new().edit(delete_four).answer("4");
        let (result, applied) = run(&titles, false, &mut host);
        assert_eq!(result.unwrap(), "Applied 4 changes: 4 deleted.");
        assert_eq!(applied.len(), 4);
    }

    #[test]
    fn a_conflict_writes_nothing_and_keeps_the_edits() {
        let mut host = Scripted::new().edit(check("Oat milk")).answer("y");
        let (result, applied) = run(&["Oat milk"], true, &mut host);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("changed in Reminders since nib last read them")
        );
        assert_eq!(applied, Vec::<ReminderChange>::new());
        assert!(
            host.said
                .iter()
                .any(|s| s.starts_with("Your edits are still in"))
        );
    }
}

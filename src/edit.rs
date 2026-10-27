use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use jiff::Zoned;
use miette::{IntoDiagnostic, Result, miette};
use serde_json::json;

use crate::backend::{Backends, ReminderChange};
use crate::host::Host;
use crate::markdown::{self, Plan, Session};
use crate::output::clean;

const INSTANT: Duration = Duration::from_millis(500);

pub fn tasks(
    list: Option<&str>,
    now: &Zoned,
    backends: &Backends,
    host: &mut dyn Host,
) -> Result<String> {
    let snapshot = backends.reminders.snapshot(list)?;
    let session = Session::new(&snapshot);
    if session.skipped > 0 {
        host.say(&format!(
            "Skipping {} with line breaks or control characters in the text; nib won't show or change {}.",
            count(session.skipped, "reminder"),
            if session.skipped == 1 { "it" } else { "them" }
        ));
    }
    let original = markdown::render(&session.document);
    let path = temp_path();
    fs::write(&path, &original).into_diagnostic()?;
    loop {
        let elapsed = host.edit(&path).into_diagnostic()?;
        let edited = fs::read_to_string(&path).into_diagnostic()?;
        if edited == original {
            if elapsed < INSTANT {
                host.say("Your editor returned right away. If it opens a window, make it wait, for example EDITOR=\"code --wait\".");
            }
            let _ = fs::remove_file(&path);
            return Ok("No changes.".into());
        }
        let plan = match markdown::parse(&edited)
            .and_then(|document| markdown::plan(&snapshot, &session, &document))
        {
            Ok(plan) => plan,
            Err(e) => {
                host.say(&clean(&e.to_string()));
                if edit_again(host)? {
                    continue;
                }
                return Err(miette!(
                    "Nothing was changed. Your edits are in {}",
                    path.display()
                ));
            }
        };
        if plan.changes.is_empty() {
            for warning in &plan.warnings {
                host.say(&clean(warning));
            }
            let _ = fs::remove_file(&path);
            return Ok("No changes.".into());
        }
        host.say(&clean(&markdown::describe(&plan, &snapshot)));
        if !confirmed(host, &plan)? {
            return Ok(format!(
                "Nothing was changed. Your edits are in {}",
                path.display()
            ));
        }
        if let Some(contents) = backup(&plan) {
            let name = format!("{}-tasks.json", now.strftime("%Y%m%dT%H%M%S"));
            let saved = host.save_backup(&name, &contents).into_diagnostic()?;
            host.say(&format!(
                "Backed up the reminders being changed to {}",
                saved.display()
            ));
        }
        if let Err(e) = backends.reminders.apply(&plan.changes) {
            host.say(&format!("Your edits are still in {}", path.display()));
            return Err(e.into());
        }
        let _ = fs::remove_file(&path);
        return Ok(summary(&plan));
    }
}

fn temp_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "nib-tasks-{}-{}.md",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn edit_again(host: &mut dyn Host) -> Result<bool> {
    let answer = host.ask("Edit again? [Y/n] ").into_diagnostic()?;
    Ok(answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn confirmed(host: &mut dyn Host, plan: &Plan) -> Result<bool> {
    let deletions = plan.deletions();
    if deletions > 3 {
        let answer = host
            .ask(&format!(
                "This deletes {deletions} reminders. Type {deletions} to apply: "
            ))
            .into_diagnostic()?;
        return Ok(answer == deletions.to_string());
    }
    let answer = host
        .ask(&format!(
            "Apply {}? [y/N] ",
            count(plan.changes.len(), "change")
        ))
        .into_diagnostic()?;
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

fn backup(plan: &Plan) -> Option<String> {
    let items: Vec<_> = plan
        .changes
        .iter()
        .filter_map(|change| match change {
            ReminderChange::Add { .. } => None,
            ReminderChange::Update { before, .. } => Some(("update", before)),
            ReminderChange::Delete { before } => Some(("delete", before)),
        })
        .map(|(change, before)| {
            json!({
                "change": change,
                "id": before.id,
                "list_id": before.list_id,
                "title": before.title,
                "notes": before.notes,
                "due": before.due.map(|d| d.to_string()),
                "priority": before.priority.map(|p| p.as_str()),
                "modified": before.modified.map(|t| t.to_string()),
            })
        })
        .collect();
    (!items.is_empty()).then(|| serde_json::to_string_pretty(&items).unwrap_or_default())
}

fn summary(plan: &Plan) -> String {
    let (mut added, mut updated, mut completed, mut deleted) = (0, 0, 0, 0);
    for change in &plan.changes {
        match change {
            ReminderChange::Add { .. } => added += 1,
            ReminderChange::Update { edit, .. } if edit.complete => completed += 1,
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
    use std::cell::RefCell;
    use std::rc::Rc;

    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::backend::{
        BackendError, Calendar, NewReminder, Notes, Reminder, ReminderList, ReminderSnapshot,
        Reminders, StoredReminder,
    };
    use crate::host::Scripted;

    type Applied = Rc<RefCell<Vec<ReminderChange>>>;

    struct Recorder {
        snapshot: ReminderSnapshot,
        applied: Applied,
        conflict: bool,
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

        fn apply(&self, changes: &[ReminderChange]) -> Result<(), BackendError> {
            if self.conflict {
                return Err(BackendError::Conflict {
                    titles: vec!["\"Oat milk\"".into()],
                });
            }
            self.applied.borrow_mut().extend_from_slice(changes);
            Ok(())
        }
    }

    fn stored(id: &str, title: &str) -> StoredReminder {
        StoredReminder {
            id: id.into(),
            list_id: "list-1".into(),
            title: title.into(),
            notes: None,
            due: None,
            priority: None,
            modified: None,
            has_alarms: false,
            has_recurrence: false,
            has_url: false,
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
        let applied = Applied::default();
        let recorder = Recorder {
            snapshot: snapshot(titles),
            applied: applied.clone(),
            conflict,
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
        assert!(matches!(&applied[..], [ReminderChange::Update { edit, .. }] if edit.complete));
        assert_eq!(host.backups.len(), 1);
        assert_eq!(host.backups[0].0, "20261026T090000-tasks.json");
        assert!(host.backups[0].1.contains("\"Oat milk\""));
        assert!(
            host.said
                .iter()
                .any(|s| s.contains("✓ complete  \"Oat milk\""))
        );
    }

    #[test]
    fn saving_without_changes_does_nothing() {
        let mut host = Scripted::new().edit(|text| text);
        let (result, applied) = run(&["Oat milk"], false, &mut host);
        assert_eq!(result.unwrap(), "No changes.");
        assert!(applied.is_empty());
        assert!(host.backups.is_empty());
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
        assert!(applied.is_empty());
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
        assert!(applied.is_empty());
        assert!(host.backups.is_empty());
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
        assert!(applied.is_empty());
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
                .contains("changed in Reminders while you were editing")
        );
        assert!(applied.is_empty());
        assert!(
            host.said
                .iter()
                .any(|s| s.starts_with("Your edits are still in"))
        );
    }
}

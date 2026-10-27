use std::collections::HashSet;

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use miette::{IntoDiagnostic, Result, miette};
use serde::{Deserialize, Serialize};

use crate::domain::{
    ApplyResult, ChangeCounts, FieldEdit, JournalAction, JournalSummary, Priority, ReminderChange,
    ReminderDueComponents, ReminderEdit, StoredReminder,
};
use crate::infra::host::{Host, JournalFile};

const VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    version: u8,
    time: Timestamp,
    action: Action,
    #[serde(default)]
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    undo_of: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    planned_changes: Vec<PlannedChange>,
    #[serde(default)]
    changes: Vec<Change>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    TaskEdit,
    Undo,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    #[default]
    Applied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
enum PlannedChange {
    Add { title: String },
    Restore { id: String, title: String },
    Update { id: String, title: String },
    Delete { id: String, title: String },
}

struct Loaded {
    id: String,
    path: std::path::PathBuf,
    written: u128,
    entry: Entry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
enum Change {
    Added {
        written: ReminderRecord,
    },
    Updated {
        before: ReminderRecord,
        written: ReminderRecord,
    },
    Completed {
        before: ReminderRecord,
        written: ReminderRecord,
    },
    Deleted {
        backup: ReminderRecord,
    },
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReminderRecord {
    id: String,
    list_id: String,
    title: String,
    notes: Option<String>,
    due: Option<Date>,
    due_components: Option<DueRecord>,
    priority: Option<PriorityRecord>,
    completed: bool,
    modified: Option<Timestamp>,
    alarms_present: bool,
    recurrence_present: bool,
    url: Option<String>,
    partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DueRecord {
    calendar: Option<String>,
    time_zone: Option<String>,
    era: Option<i64>,
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
    hour: Option<i64>,
    minute: Option<i64>,
    second: Option<i64>,
    nanosecond: Option<i64>,
    weekday: Option<i64>,
    weekday_ordinal: Option<i64>,
    quarter: Option<i64>,
    week_of_month: Option<i64>,
    week_of_year: Option<i64>,
    year_for_week_of_year: Option<i64>,
    day_of_year: Option<i64>,
    is_leap_month: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PriorityRecord {
    Low,
    Medium,
    High,
}

impl From<Priority> for PriorityRecord {
    fn from(value: Priority) -> Self {
        match value {
            Priority::Low => Self::Low,
            Priority::Medium => Self::Medium,
            Priority::High => Self::High,
        }
    }
}

impl From<PriorityRecord> for Priority {
    fn from(value: PriorityRecord) -> Self {
        match value {
            PriorityRecord::Low => Self::Low,
            PriorityRecord::Medium => Self::Medium,
            PriorityRecord::High => Self::High,
        }
    }
}

impl From<&StoredReminder> for ReminderRecord {
    fn from(value: &StoredReminder) -> Self {
        Self {
            id: value.id.clone(),
            list_id: value.list_id.clone(),
            title: value.title.clone(),
            notes: value.notes.clone(),
            due: value.due,
            due_components: value.due_components.as_ref().map(DueRecord::from),
            priority: value.priority.map(PriorityRecord::from),
            completed: value.completed,
            modified: value.modified,
            alarms_present: value.alarms_present,
            recurrence_present: value.recurrence_present,
            url: value.url.clone(),
            partial: value.backup_partial,
        }
    }
}

impl From<ReminderRecord> for StoredReminder {
    fn from(value: ReminderRecord) -> Self {
        Self {
            id: value.id,
            list_id: value.list_id,
            title: value.title,
            notes: value.notes,
            due: value.due,
            due_components: value.due_components.map(ReminderDueComponents::from),
            priority: value.priority.map(Priority::from),
            completed: value.completed,
            modified: value.modified,
            alarms_present: value.alarms_present,
            recurrence_present: value.recurrence_present,
            url: value.url,
            backup_partial: value.partial,
        }
    }
}

impl From<&ReminderDueComponents> for DueRecord {
    fn from(value: &ReminderDueComponents) -> Self {
        Self {
            calendar: value.calendar.clone(),
            time_zone: value.time_zone.clone(),
            era: value.era,
            year: value.year,
            month: value.month,
            day: value.day,
            hour: value.hour,
            minute: value.minute,
            second: value.second,
            nanosecond: value.nanosecond,
            weekday: value.weekday,
            weekday_ordinal: value.weekday_ordinal,
            quarter: value.quarter,
            week_of_month: value.week_of_month,
            week_of_year: value.week_of_year,
            year_for_week_of_year: value.year_for_week_of_year,
            day_of_year: value.day_of_year,
            is_leap_month: value.is_leap_month,
        }
    }
}

impl From<DueRecord> for ReminderDueComponents {
    fn from(value: DueRecord) -> Self {
        Self {
            calendar: value.calendar,
            time_zone: value.time_zone,
            era: value.era,
            year: value.year,
            month: value.month,
            day: value.day,
            hour: value.hour,
            minute: value.minute,
            second: value.second,
            nanosecond: value.nanosecond,
            weekday: value.weekday,
            weekday_ordinal: value.weekday_ordinal,
            quarter: value.quarter,
            week_of_month: value.week_of_month,
            week_of_year: value.week_of_year,
            year_for_week_of_year: value.year_for_week_of_year,
            day_of_year: value.day_of_year,
            is_leap_month: value.is_leap_month,
        }
    }
}

pub fn record_edit(
    now: &Zoned,
    changes: &[ReminderChange],
    result: &ApplyResult,
    host: &mut dyn Host,
) -> Result<String> {
    record(now, Action::TaskEdit, None, changes, result, host)
}

pub fn begin_undo(
    now: &Zoned,
    undo_of: &str,
    changes: &[ReminderChange],
    host: &mut dyn Host,
) -> Result<String> {
    let entry = Entry {
        version: VERSION,
        time: now.timestamp(),
        action: Action::Undo,
        status: Status::Pending,
        undo_of: Some(undo_of.to_owned()),
        planned_changes: changes.iter().map(PlannedChange::from).collect(),
        changes: Vec::new(),
    };
    save_new(now, &entry, host)
}

pub fn finish_undo(
    id: &str,
    now: &Zoned,
    undo_of: &str,
    changes: &[ReminderChange],
    result: &ApplyResult,
    host: &mut dyn Host,
) -> Result<String> {
    let entry = applied_entry(now, Action::Undo, Some(undo_of.to_owned()), changes, result)?;
    let contents = serde_json::to_string_pretty(&entry).into_diagnostic()?;
    let path = host.replace_journal(id, &contents).into_diagnostic()?;
    Ok(path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(id)
        .to_owned())
}

fn record(
    now: &Zoned,
    action: Action,
    undo_of: Option<String>,
    changes: &[ReminderChange],
    result: &ApplyResult,
    host: &mut dyn Host,
) -> Result<String> {
    let entry = applied_entry(now, action, undo_of, changes, result)?;
    save_new(now, &entry, host)
}

fn applied_entry(
    now: &Zoned,
    action: Action,
    undo_of: Option<String>,
    changes: &[ReminderChange],
    result: &ApplyResult,
) -> Result<Entry> {
    if changes.len() != result.written.len() {
        return Err(miette!("Reminders returned an incomplete apply result"));
    }
    let mut journal_changes = Vec::with_capacity(changes.len());
    for (change, written) in changes.iter().zip(&result.written) {
        journal_changes.push(match change {
            ReminderChange::Add { .. } | ReminderChange::Restore { .. } => Change::Added {
                written: ReminderRecord::from(
                    written
                        .as_ref()
                        .ok_or_else(|| miette!("Reminders didn't return an added reminder"))?,
                ),
            },
            ReminderChange::Update { before, edit } => {
                let written = ReminderRecord::from(
                    written
                        .as_ref()
                        .ok_or_else(|| miette!("Reminders didn't return an updated reminder"))?,
                );
                if edit.completed == Some(true) {
                    Change::Completed {
                        before: ReminderRecord::from(before),
                        written,
                    }
                } else {
                    Change::Updated {
                        before: ReminderRecord::from(before),
                        written,
                    }
                }
            }
            ReminderChange::Delete { before } => Change::Deleted {
                backup: ReminderRecord::from(before),
            },
        });
    }
    Ok(Entry {
        version: VERSION,
        time: now.timestamp(),
        action,
        status: Status::Applied,
        undo_of,
        planned_changes: Vec::new(),
        changes: journal_changes,
    })
}

fn save_new(now: &Zoned, entry: &Entry, host: &mut dyn Host) -> Result<String> {
    let contents = serde_json::to_string_pretty(&entry).into_diagnostic()?;
    let name = format!("{}-journal.json", now.strftime("%Y%m%dT%H%M%S"));
    let path = host.save_journal(&name, &contents).into_diagnostic()?;
    Ok(path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&name)
        .to_owned())
}

impl From<&ReminderChange> for PlannedChange {
    fn from(value: &ReminderChange) -> Self {
        match value {
            ReminderChange::Add { title, .. } => Self::Add {
                title: title.clone(),
            },
            ReminderChange::Restore { before } => Self::Restore {
                id: before.id.clone(),
                title: before.title.clone(),
            },
            ReminderChange::Update { before, .. } => Self::Update {
                id: before.id.clone(),
                title: before.title.clone(),
            },
            ReminderChange::Delete { before } => Self::Delete {
                id: before.id.clone(),
                title: before.title.clone(),
            },
        }
    }
}

pub fn summaries(host: &mut dyn Host) -> Result<Vec<JournalSummary>> {
    let mut entries = Vec::new();
    for file in host.journal_files().into_diagnostic()? {
        match parse(file) {
            Ok(entry) => entries.push(entry),
            Err(error) => host.say(&format!("warning: {error}")),
        }
    }
    sort_entries(&mut entries);
    let inactive = inactive_entries(&entries);
    Ok(entries
        .into_iter()
        .map(|loaded| {
            let entry_counts = counts(&loaded.entry);
            JournalSummary {
                undone: inactive.contains(&loaded.id),
                id: loaded.id,
                time: loaded.entry.time,
                action: action(loaded.entry.action),
                undo_of: loaded.entry.undo_of,
                counts: entry_counts,
            }
        })
        .collect())
}

pub fn undo_changes(
    wanted: Option<&str>,
    host: &mut dyn Host,
) -> Result<(String, Vec<ReminderChange>, ChangeCounts, bool)> {
    let mut entries = load(host)?;
    sort_entries(&mut entries);
    if let Some(pending) = entries
        .iter()
        .find(|loaded| loaded.entry.status == Status::Pending)
    {
        return Err(miette!(
            "undo journal entry {:?} is still pending; inspect {} before trying another undo",
            pending.id,
            pending.path.display()
        ));
    }
    let inactive = inactive_entries(&entries);
    let loaded = match wanted {
        Some(wanted) => {
            let found = entries
                .into_iter()
                .find(|loaded| loaded.id == wanted)
                .ok_or_else(|| miette!("journal entry {wanted:?} not found"))?;
            if inactive.contains(wanted) {
                return Err(miette!("journal entry {wanted:?} has already been undone"));
            }
            found
        }
        None => entries
            .into_iter()
            .find(|loaded| {
                loaded.entry.action == Action::TaskEdit && !inactive.contains(&loaded.id)
            })
            .ok_or_else(|| miette!("there are no journal entries to undo"))?,
    };
    let id = loaded.id;
    let entry = loaded.entry;
    let partial = entry.changes.iter().any(|change| match change {
        Change::Deleted { backup } => backup.partial,
        _ => false,
    });
    let changes = entry
        .changes
        .into_iter()
        .map(|change| match change {
            Change::Added { written } => ReminderChange::Delete {
                before: written.into(),
            },
            Change::Updated { before, written } | Change::Completed { before, written } => {
                let before: StoredReminder = before.into();
                let written: StoredReminder = written.into();
                ReminderChange::Update {
                    edit: restore_edit(&written, &before),
                    before: written,
                }
            }
            Change::Deleted { backup } => ReminderChange::Restore {
                before: backup.into(),
            },
        })
        .collect::<Vec<_>>();
    let undo_counts = change_counts(&changes);
    Ok((id, changes, undo_counts, partial))
}

fn load(host: &mut dyn Host) -> Result<Vec<Loaded>> {
    host.journal_files()
        .into_diagnostic()?
        .into_iter()
        .map(parse)
        .collect()
}

fn parse(file: JournalFile) -> Result<Loaded> {
    let contents = file.contents.map_err(|error| {
        miette!(
            "couldn't read journal entry {}: {error}",
            file.path.display()
        )
    })?;
    let entry: Entry = serde_json::from_str(&contents).map_err(|error| {
        miette!(
            "couldn't read journal entry {}: {error}",
            file.path.display()
        )
    })?;
    if entry.version != VERSION {
        return Err(miette!(
            "journal entry {} uses unsupported version {}",
            file.path.display(),
            entry.version
        ));
    }
    Ok(Loaded {
        id: file.id,
        path: file.path,
        written: file.written,
        entry,
    })
}

fn sort_entries(entries: &mut [Loaded]) {
    entries.sort_by(|a, b| (b.written, &b.id).cmp(&(a.written, &a.id)));
}

fn inactive_entries(entries: &[Loaded]) -> HashSet<String> {
    let mut inactive = HashSet::new();
    for loaded in entries {
        if !inactive.contains(&loaded.id)
            && let Some(target) = &loaded.entry.undo_of
        {
            inactive.insert(target.clone());
        }
    }
    inactive
}

fn action(value: Action) -> JournalAction {
    match value {
        Action::TaskEdit => JournalAction::TaskEdit,
        Action::Undo => JournalAction::Undo,
    }
}

fn counts(entry: &Entry) -> ChangeCounts {
    if entry.status == Status::Pending {
        return planned_counts(&entry.planned_changes);
    }
    let mut counts = ChangeCounts::default();
    for change in &entry.changes {
        match change {
            Change::Added { .. } => counts.added += 1,
            Change::Updated { .. } => counts.updated += 1,
            Change::Completed { .. } => counts.completed += 1,
            Change::Deleted { .. } => counts.deleted += 1,
        }
    }
    counts
}

fn planned_counts(changes: &[PlannedChange]) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for change in changes {
        match change {
            PlannedChange::Add { .. } | PlannedChange::Restore { .. } => counts.added += 1,
            PlannedChange::Update { .. } => counts.updated += 1,
            PlannedChange::Delete { .. } => counts.deleted += 1,
        }
    }
    counts
}

fn change_counts(changes: &[ReminderChange]) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for change in changes {
        match change {
            ReminderChange::Add { .. } | ReminderChange::Restore { .. } => counts.added += 1,
            ReminderChange::Update { edit, .. } if edit.completed == Some(true) => {
                counts.completed += 1;
            }
            ReminderChange::Update { .. } => counts.updated += 1,
            ReminderChange::Delete { .. } => counts.deleted += 1,
        }
    }
    counts
}

fn restore_edit(current: &StoredReminder, original: &StoredReminder) -> ReminderEdit {
    ReminderEdit {
        title: (current.title != original.title).then(|| original.title.clone()),
        notes: field_edit(current.notes.as_ref(), original.notes.as_ref()),
        due: field_edit(current.due.as_ref(), original.due.as_ref()),
        due_components: (current.due_components != original.due_components)
            .then(|| original.due_components.clone().map(Box::new))
            .flatten(),
        priority: field_edit(current.priority.as_ref(), original.priority.as_ref()),
        list_id: (current.list_id != original.list_id).then(|| original.list_id.clone()),
        completed: (current.completed != original.completed).then_some(original.completed),
    }
}

fn field_edit<T: Clone + Eq>(current: Option<&T>, original: Option<&T>) -> FieldEdit<T> {
    if current == original {
        FieldEdit::Unchanged
    } else {
        original.cloned().map_or(FieldEdit::Remove, FieldEdit::Set)
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::infra::host::Scripted;

    fn now() -> Zoned {
        at(9)
    }

    fn at(hour: i8) -> Zoned {
        date(2026, 10, 26)
            .at(hour, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn reminder(id: &str, title: &str) -> StoredReminder {
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
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    #[test]
    fn applied_values_and_new_identifiers_are_journaled() {
        let before_update = reminder("update-1", "Old title");
        let mut written_update = before_update.clone();
        written_update.title = "New title".into();
        let before_complete = reminder("complete-1", "Finish report");
        let mut written_complete = before_complete.clone();
        written_complete.completed = true;
        let deleted = reminder("delete-1", "Old reminder");
        let added = reminder("new-apple-id", "New reminder");
        let changes = vec![
            ReminderChange::Add {
                list_id: "list-1".into(),
                title: "New reminder".into(),
                notes: None,
                due: None,
                priority: None,
            },
            ReminderChange::Update {
                before: before_update,
                edit: ReminderEdit {
                    title: Some("New title".into()),
                    ..ReminderEdit::default()
                },
            },
            ReminderChange::Update {
                before: before_complete,
                edit: ReminderEdit {
                    completed: Some(true),
                    ..ReminderEdit::default()
                },
            },
            ReminderChange::Delete { before: deleted },
        ];
        let result = ApplyResult {
            written: vec![
                Some(added),
                Some(written_update),
                Some(written_complete),
                None,
            ],
        };
        let mut host = Scripted::new();
        let id = record_edit(&now(), &changes, &result, &mut host).unwrap();
        assert_eq!(id, "20261026T090000-journal");
        let value: serde_json::Value = serde_json::from_str(&host.journals[0].1).unwrap();
        assert_eq!(value["changes"][0]["written"]["id"], "new-apple-id");
        assert_eq!(value["changes"][1]["before"]["title"], "Old title");
        assert_eq!(value["changes"][1]["written"]["title"], "New title");
        assert_eq!(value["changes"][2]["change"], "completed");
        assert_eq!(value["changes"][3]["backup"]["id"], "delete-1");
        assert_eq!(
            summaries(&mut host).unwrap()[0].counts,
            ChangeCounts {
                added: 1,
                updated: 1,
                completed: 1,
                deleted: 1,
            }
        );
    }

    #[test]
    fn undo_inverts_every_change_and_uses_the_latest_entry() {
        let before = reminder("complete-1", "Finish report");
        let mut written = before.clone();
        written.completed = true;
        let added = reminder("new-apple-id", "New reminder");
        let deleted = reminder("delete-1", "Old reminder");
        let changes = vec![
            ReminderChange::Add {
                list_id: "list-1".into(),
                title: added.title.clone(),
                notes: None,
                due: None,
                priority: None,
            },
            ReminderChange::Update {
                before,
                edit: ReminderEdit {
                    completed: Some(true),
                    ..ReminderEdit::default()
                },
            },
            ReminderChange::Delete {
                before: deleted.clone(),
            },
        ];
        let result = ApplyResult {
            written: vec![Some(added.clone()), Some(written.clone()), None],
        };
        let mut host = Scripted::new();
        record_edit(&now(), &changes, &result, &mut host).unwrap();
        let (id, inverse, counts, partial) = undo_changes(None, &mut host).unwrap();
        assert_eq!(id, "20261026T090000-journal");
        assert_eq!(
            counts,
            ChangeCounts {
                added: 1,
                updated: 1,
                completed: 0,
                deleted: 1,
            }
        );
        assert!(!partial);
        assert!(matches!(
            &inverse[0],
            ReminderChange::Delete { before } if before == &added
        ));
        assert!(matches!(
            &inverse[1],
            ReminderChange::Update { before, edit }
                if before == &written && edit.completed == Some(false)
        ));
        assert!(matches!(
            &inverse[2],
            ReminderChange::Restore { before } if before == &deleted
        ));
    }

    #[test]
    fn an_entry_cannot_be_undone_twice() {
        let deleted = reminder("delete-1", "Old reminder");
        let original = ReminderChange::Delete {
            before: deleted.clone(),
        };
        let mut host = Scripted::new();
        let original_id = record_edit(
            &now(),
            &[original],
            &ApplyResult {
                written: vec![None],
            },
            &mut host,
        )
        .unwrap();
        let inverse = ReminderChange::Restore { before: deleted };
        let restored = reminder("new-id", "Old reminder");
        let pending_id = begin_undo(
            &at(10),
            &original_id,
            std::slice::from_ref(&inverse),
            &mut host,
        )
        .unwrap();
        finish_undo(
            &pending_id,
            &at(10),
            &original_id,
            &[inverse],
            &ApplyResult {
                written: vec![Some(restored)],
            },
            &mut host,
        )
        .unwrap();
        let error = undo_changes(Some(&original_id), &mut host).unwrap_err();
        assert!(error.to_string().contains("already been undone"));
    }

    #[test]
    fn undoing_due_date_removal_restores_full_components() {
        let mut original = reminder("reminder-1", "Call Chicago");
        original.due = Some(date(2026, 10, 27));
        original.due_components = Some(ReminderDueComponents {
            calendar: Some("gregorian".into()),
            time_zone: Some("America/Chicago".into()),
            year: Some(2026),
            month: Some(10),
            day: Some(27),
            hour: Some(14),
            minute: Some(30),
            ..ReminderDueComponents::default()
        });
        let mut current = original.clone();
        current.due = None;
        current.due_components = None;

        let edit = restore_edit(&current, &original);
        assert_eq!(edit.due, FieldEdit::Set(date(2026, 10, 27)));
        assert_eq!(
            edit.due_components.as_deref(),
            original.due_components.as_ref()
        );
    }

    #[test]
    fn plain_undo_skips_undo_entries_but_explicit_id_can_reverse_one() {
        let first = reminder("first", "First deletion");
        let second = reminder("second", "Second deletion");
        let mut host = Scripted::new();
        let first_id = record_edit(
            &at(8),
            &[ReminderChange::Delete {
                before: first.clone(),
            }],
            &ApplyResult {
                written: vec![None],
            },
            &mut host,
        )
        .unwrap();
        let second_id = record_edit(
            &at(9),
            &[ReminderChange::Delete {
                before: second.clone(),
            }],
            &ApplyResult {
                written: vec![None],
            },
            &mut host,
        )
        .unwrap();
        let inverse = ReminderChange::Restore { before: second };
        let undo_id = begin_undo(
            &at(10),
            &second_id,
            std::slice::from_ref(&inverse),
            &mut host,
        )
        .unwrap();
        finish_undo(
            &undo_id,
            &at(10),
            &second_id,
            &[inverse],
            &ApplyResult {
                written: vec![Some(reminder("second-new", "Second deletion"))],
            },
            &mut host,
        )
        .unwrap();

        assert_eq!(undo_changes(None, &mut host).unwrap().0, first_id);
        let (_, changes, _, _) = undo_changes(Some(&undo_id), &mut host).unwrap();
        assert!(matches!(
            &changes[0],
            ReminderChange::Delete { before } if before.id == "second-new"
        ));
    }

    #[test]
    fn latest_entry_uses_host_write_order_not_command_start_time() {
        let mut host = Scripted::new();
        record_edit(
            &at(10),
            &[ReminderChange::Delete {
                before: reminder("slow", "Slow edit"),
            }],
            &ApplyResult {
                written: vec![None],
            },
            &mut host,
        )
        .unwrap();
        let committed_last = record_edit(
            &at(9),
            &[ReminderChange::Delete {
                before: reminder("fast", "Committed last"),
            }],
            &ApplyResult {
                written: vec![None],
            },
            &mut host,
        )
        .unwrap();

        assert_eq!(undo_changes(None, &mut host).unwrap().0, committed_last);
    }

    #[test]
    fn log_skips_unreadable_journals_but_undo_fails_closed() {
        let mut source = Scripted::new();
        record_edit(
            &now(),
            &[ReminderChange::Delete {
                before: reminder("valid", "Valid entry"),
            }],
            &ApplyResult {
                written: vec![None],
            },
            &mut source,
        )
        .unwrap();
        let contents = source.journals[0].1.clone();
        let mut host = Scripted::new()
            .journal("valid-journal", &contents)
            .unreadable_journal("broken-journal", "permission denied");

        let entries = summaries(&mut host).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(host.said[0].contains("/backups/broken-journal.json"));
        let error = undo_changes(None, &mut host).unwrap_err().to_string();
        assert!(error.contains("/backups/broken-journal.json"));
    }
}

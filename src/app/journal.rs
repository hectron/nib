use std::collections::HashSet;

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use miette::{IntoDiagnostic, Result, miette};
use serde::{Deserialize, Serialize};

use crate::domain::{
    ApplyResult, ChangeCounts, FieldEdit, JournalSummary, Priority, ReminderChange,
    ReminderDueComponents, ReminderEdit, StoredReminder,
};
use crate::infra::host::{Host, JournalFile};

const VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    version: u8,
    time: Timestamp,
    action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    undo_of: Option<String>,
    changes: Vec<Change>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    TaskEdit,
    Undo,
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

pub fn record_undo(
    now: &Zoned,
    undo_of: &str,
    changes: &[ReminderChange],
    result: &ApplyResult,
    host: &mut dyn Host,
) -> Result<String> {
    record(
        now,
        Action::Undo,
        Some(undo_of.to_owned()),
        changes,
        result,
        host,
    )
}

fn record(
    now: &Zoned,
    action: Action,
    undo_of: Option<String>,
    changes: &[ReminderChange],
    result: &ApplyResult,
    host: &mut dyn Host,
) -> Result<String> {
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
    let entry = Entry {
        version: VERSION,
        time: now.timestamp(),
        action,
        undo_of,
        changes: journal_changes,
    };
    let contents = serde_json::to_string_pretty(&entry).into_diagnostic()?;
    let name = format!("{}-journal.json", now.strftime("%Y%m%dT%H%M%S"));
    let path = host.save_journal(&name, &contents).into_diagnostic()?;
    Ok(path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&name)
        .to_owned())
}

pub fn summaries(host: &mut dyn Host) -> Result<Vec<JournalSummary>> {
    let mut entries = load(host)?;
    entries.sort_by(|a, b| (b.1.time, &b.0).cmp(&(a.1.time, &a.0)));
    Ok(entries
        .into_iter()
        .map(|(id, entry)| JournalSummary {
            id,
            time: entry.time,
            counts: counts(&entry),
        })
        .collect())
}

pub fn undo_changes(
    wanted: Option<&str>,
    host: &mut dyn Host,
) -> Result<(String, Vec<ReminderChange>, ChangeCounts, bool)> {
    let mut entries = load(host)?;
    entries.sort_by(|a, b| (b.1.time, &b.0).cmp(&(a.1.time, &a.0)));
    let mut inactive = HashSet::new();
    for (id, entry) in &entries {
        if !inactive.contains(id)
            && let Some(target) = &entry.undo_of
        {
            inactive.insert(target.clone());
        }
    }
    let (id, entry) = match wanted {
        Some(wanted) => {
            let found = entries
                .into_iter()
                .find(|(id, _)| id == wanted)
                .ok_or_else(|| miette!("journal entry {wanted:?} not found"))?;
            if inactive.contains(wanted) {
                return Err(miette!("journal entry {wanted:?} has already been undone"));
            }
            found
        }
        None => entries
            .into_iter()
            .find(|(id, _)| !inactive.contains(id))
            .ok_or_else(|| miette!("there are no journal entries to undo"))?,
    };
    let partial = entry.changes.iter().any(|change| match change {
        Change::Deleted { backup } => backup.partial,
        _ => false,
    });
    let entry_counts = counts(&entry);
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
        .collect();
    Ok((id, changes, entry_counts, partial))
}

fn load(host: &mut dyn Host) -> Result<Vec<(String, Entry)>> {
    host.journal_files()
        .into_diagnostic()?
        .into_iter()
        .map(parse)
        .collect()
}

fn parse(file: JournalFile) -> Result<(String, Entry)> {
    let entry: Entry = serde_json::from_str(&file.contents)
        .map_err(|error| miette!("couldn't read journal entry {:?}: {error}", file.id))?;
    if entry.version != VERSION {
        return Err(miette!(
            "journal entry {:?} uses unsupported version {}",
            file.id,
            entry.version
        ));
    }
    Ok((file.id, entry))
}

fn counts(entry: &Entry) -> ChangeCounts {
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

fn restore_edit(current: &StoredReminder, original: &StoredReminder) -> ReminderEdit {
    ReminderEdit {
        title: (current.title != original.title).then(|| original.title.clone()),
        notes: field_edit(current.notes.as_ref(), original.notes.as_ref()),
        due: field_edit(current.due.as_ref(), original.due.as_ref()),
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
        assert_eq!(counts.total(), 3);
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
        record_undo(
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
}

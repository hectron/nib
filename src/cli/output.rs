use comfy_table::{
    Attribute, Cell, ColumnConstraint, ContentArrangement, Row, Table, presets::NOTHING,
};
use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use miette::{IntoDiagnostic, Result};
use serde::{Serialize, Serializer};

use crate::domain::{ChangeCounts, Event, JournalSummary, Note, Priority, Reminder};

pub trait Tabular: Serialize {
    const HEADER: &'static [&'static str];
    const TITLE_COLUMN: usize;
    fn row(&self) -> Vec<String>;
}

pub trait Summary: Serialize {
    fn title(&self) -> &str;
    fn detail(&self) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Style {
    pub color: bool,
    pub width: Option<u16>,
}

impl Style {
    pub fn detect() -> Self {
        let probe = Table::new();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self {
            color: probe.is_tty() && !no_color,
            width: probe.width(),
        }
    }
}

pub fn list<T: Tabular>(items: &[T], json: bool, style: Style) -> Result<String> {
    if json {
        return to_json(items);
    }
    if items.is_empty() {
        return Ok(String::new());
    }
    let mut table = Table::new();
    table
        .force_no_tty()
        .load_style(NOTHING)
        .set_truncation_indicator("…");
    if style.color {
        table.enforce_styling().style_text_only();
    }
    if let Some(width) = style.width {
        table
            .set_width(width)
            .set_content_arrangement(ContentArrangement::Dynamic);
    }
    let mut header = Row::from(
        T::HEADER
            .iter()
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    header.max_height(1);
    table.set_header(header);
    for item in items {
        let mut row = Row::from(item.row().iter().map(|cell| clean(cell)));
        row.max_height(1);
        table.add_row(row);
    }
    for (i, column) in table.column_iter_mut().enumerate() {
        column.set_padding((0, 2));
        if i != T::TITLE_COLUMN {
            column.set_constraint(ColumnConstraint::ContentWidth);
        }
    }
    Ok(table.trim_fmt())
}

pub fn single<T: Summary>(item: &T, verb: &str, json: bool) -> Result<String> {
    if json {
        to_json(item)
    } else {
        let detail = item.detail().map(|d| format!(" {d}")).unwrap_or_default();
        Ok(clean(&format!("{verb} \"{}\"{detail}", item.title())))
    }
}

pub fn today(
    overdue: &[Reminder],
    due_today: &[Reminder],
    events: &[Event],
    json: bool,
    style: Style,
) -> Result<String> {
    if json {
        return to_json(&TodayJson {
            overdue,
            due_today,
            events,
        });
    }
    let sections = [
        ("Overdue", list(overdue, false, style)?),
        ("Due today", list(due_today, false, style)?),
        ("Events", list(events, false, style)?),
    ];
    let rendered = sections
        .into_iter()
        .filter(|(_, table)| !table.is_empty())
        .map(|(heading, table)| format!("{}\n{table}", styled_heading(heading, style)))
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        Ok("Nothing due today.".into())
    } else {
        Ok(rendered.join("\n\n"))
    }
}

pub fn journal(entries: &[JournalSummary], json: bool, style: Style) -> Result<String> {
    list(entries, json, style)
}

pub fn undo(id: &str, undo_id: &str, counts: ChangeCounts, json: bool) -> Result<String> {
    if json {
        return to_json(&UndoJson {
            id,
            undo_id: Some(undo_id),
            applied: true,
            added: counts.added,
            updated: counts.updated,
            completed: counts.completed,
            deleted: counts.deleted,
        });
    }
    Ok(format!(
        "Undid {} from {id}.",
        count(counts.total(), "change")
    ))
}

pub fn undo_cancelled(id: &str, json: bool) -> Result<String> {
    if json {
        return to_json(&UndoJson {
            id,
            undo_id: None,
            applied: false,
            added: 0,
            updated: 0,
            completed: 0,
            deleted: 0,
        });
    }
    Ok("Nothing was changed.".into())
}

pub fn undo_resolution(id: &str, applied: bool, json: bool) -> Result<String> {
    if json {
        return to_json(&UndoResolutionJson {
            id,
            resolved: true,
            applied,
        });
    }
    if applied {
        Ok(format!("Marked pending undo {id} as applied."))
    } else {
        Ok(format!("Removed pending undo {id}."))
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn styled_heading(heading: &str, style: Style) -> String {
    if style.color {
        format!("\u{1b}[1m{heading}\u{1b}[0m")
    } else {
        heading.into()
    }
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).into_diagnostic()
}

pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => '\u{fffd}',
            c => c,
        })
        .collect()
}

pub fn clean_lines(text: &str) -> String {
    text.lines().map(clean).collect::<Vec<_>>().join("\n")
}

impl Tabular for Note {
    const HEADER: &'static [&'static str] = &["TITLE", "FOLDER", "LOCKED"];
    const TITLE_COLUMN: usize = 0;

    fn row(&self) -> Vec<String> {
        vec![
            self.title.clone(),
            self.folder.clone().unwrap_or_default(),
            if self.locked {
                "yes".into()
            } else {
                String::new()
            },
        ]
    }
}

impl Summary for Note {
    fn title(&self) -> &str {
        &self.title
    }

    fn detail(&self) -> Option<String> {
        self.folder.as_ref().map(|folder| format!("in {folder}"))
    }
}

impl Tabular for Reminder {
    const HEADER: &'static [&'static str] = &["TITLE", "DUE", "PRIORITY"];
    const TITLE_COLUMN: usize = 0;

    fn row(&self) -> Vec<String> {
        vec![
            self.title.clone(),
            self.due.map(|d| d.to_string()).unwrap_or_default(),
            self.priority
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
        ]
    }
}

impl Summary for Reminder {
    fn title(&self) -> &str {
        &self.title
    }

    fn detail(&self) -> Option<String> {
        let details: Vec<String> = [
            self.due.map(|d| format!("due {d}")),
            self.priority.map(|p| format!("{} priority", p.as_str())),
        ]
        .into_iter()
        .flatten()
        .collect();
        if details.is_empty() {
            None
        } else {
            Some(format!("({})", details.join(", ")))
        }
    }
}

impl Tabular for Event {
    const HEADER: &'static [&'static str] = &["START", "TITLE"];
    const TITLE_COLUMN: usize = 1;

    fn row(&self) -> Vec<String> {
        let start = if self.all_day {
            self.start.strftime("%Y-%m-%d all day")
        } else {
            self.start.strftime("%Y-%m-%d %H:%M")
        };
        vec![start.to_string(), self.title.clone()]
    }
}

#[derive(Serialize)]
struct NoteJson<'a> {
    id: &'a str,
    title: &'a str,
    folder: Option<&'a str>,
    modified: Option<Timestamp>,
    locked: bool,
}

impl Serialize for Note {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        NoteJson {
            id: &self.id,
            title: &self.title,
            folder: self.folder.as_deref(),
            modified: self.modified,
            locked: self.locked,
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct ReminderJson<'a> {
    id: &'a str,
    title: &'a str,
    due: Option<Date>,
    priority: Option<&'static str>,
}

impl Serialize for Reminder {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ReminderJson {
            id: &self.id,
            title: &self.title,
            due: self.due,
            priority: self.priority.map(Priority::as_str),
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct EventJson<'a> {
    id: &'a str,
    title: &'a str,
    #[serde(serialize_with = "rfc3339")]
    start: &'a Zoned,
    all_day: bool,
}

#[derive(Serialize)]
struct TodayJson<'a> {
    overdue: &'a [Reminder],
    due_today: &'a [Reminder],
    events: &'a [Event],
}

#[derive(Serialize)]
struct JournalJson<'a> {
    id: &'a str,
    time: Timestamp,
    written_at: Timestamp,
    action: &'static str,
    status: &'static str,
    undo_of: Option<&'a str>,
    undone: bool,
    added: usize,
    updated: usize,
    completed: usize,
    deleted: usize,
}

#[derive(Serialize)]
struct UndoResolutionJson<'a> {
    id: &'a str,
    resolved: bool,
    applied: bool,
}

#[derive(Serialize)]
struct UndoJson<'a> {
    id: &'a str,
    undo_id: Option<&'a str>,
    applied: bool,
    added: usize,
    updated: usize,
    completed: usize,
    deleted: usize,
}

impl Serialize for JournalSummary {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        JournalJson {
            id: &self.id,
            time: self.time,
            written_at: self.written_at,
            action: self.action.as_str(),
            status: self.status.as_str(),
            undo_of: self.undo_of.as_deref(),
            undone: self.undone,
            added: self.counts.added,
            updated: self.counts.updated,
            completed: self.counts.completed,
            deleted: self.counts.deleted,
        }
        .serialize(serializer)
    }
}

impl Tabular for JournalSummary {
    const HEADER: &'static [&'static str] = &[
        "ENTRY",
        "WRITTEN",
        "ACTION",
        "STATUS",
        "UNDO OF",
        "UNDONE",
        "ADDED",
        "UPDATED",
        "COMPLETED",
        "DELETED",
    ];
    const TITLE_COLUMN: usize = 0;

    fn row(&self) -> Vec<String> {
        vec![
            self.id.clone(),
            self.written_at.to_string(),
            self.action.as_str().into(),
            self.status.as_str().into(),
            self.undo_of.clone().unwrap_or_default(),
            if self.undone {
                "yes".into()
            } else {
                String::new()
            },
            self.counts.added.to_string(),
            self.counts.updated.to_string(),
            self.counts.completed.to_string(),
            self.counts.deleted.to_string(),
        ]
    }
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        EventJson {
            id: &self.id,
            title: &self.title,
            start: &self.start,
            all_day: self.all_day,
        }
        .serialize(serializer)
    }
}

fn rfc3339<S: Serializer>(time: &Zoned, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&time.timestamp().display_with_offset(time.offset()))
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use jiff::civil::date;

    use super::*;

    fn reminders() -> Vec<Reminder> {
        vec![
            Reminder {
                id: "reminder-1".into(),
                title: "Book flights to Lisbon for the November offsite".into(),
                due: Some(date(2026, 10, 27)),
                priority: Some(Priority::High),
            },
            Reminder {
                id: "reminder-2".into(),
                title: "Buy groceries".into(),
                due: None,
                priority: None,
            },
        ]
    }

    #[test]
    fn rows_stay_whole_without_a_width() {
        assert_snapshot!(list(&reminders(), false, Style::default()).unwrap(), @r"
        TITLE                                            DUE         PRIORITY
        Book flights to Lisbon for the November offsite  2026-10-27  high
        Buy groceries
        ");
    }

    #[test]
    fn long_titles_truncate_to_the_width() {
        let style = Style {
            color: false,
            width: Some(40),
        };
        assert_snapshot!(list(&reminders(), false, style).unwrap(), @r"
        TITLE             DUE         PRIORITY
        Book flights to…  2026-10-27  high
        Buy groceries
        ");
    }

    #[test]
    fn color_bolds_only_the_header_text() {
        let style = Style {
            color: true,
            width: None,
        };
        let colored = list(&reminders(), false, style).unwrap();
        let plain = list(&reminders(), false, Style::default()).unwrap();
        let (header, rows) = colored.split_once('\n').unwrap();
        assert!(header.starts_with("\u{1b}[1mTITLE\u{1b}[0m "), "{header:?}");
        assert!(header.ends_with("\u{1b}[1mPRIORITY\u{1b}[0m"), "{header:?}");
        assert_eq!(
            header.replace("\u{1b}[1m", "").replace("\u{1b}[0m", ""),
            plain.lines().next().unwrap()
        );
        assert!(!rows.contains('\u{1b}'));
    }

    #[test]
    fn control_characters_never_reach_the_terminal() {
        let note = Note {
            id: "note-1".into(),
            title: "Hi\u{1b}]0;pwned\u{7}\nthere".into(),
            folder: None,
            modified: None,
            locked: false,
        };
        assert_snapshot!(
            single(&note, "Created note", false).unwrap(),
            @r#"Created note "Hi�]0;pwned� there""#
        );
        assert_snapshot!(list(std::slice::from_ref(&note), false, Style::default()).unwrap(), @r"
        TITLE               FOLDER  LOCKED
        Hi�]0;pwned� there
        ");
        assert!(
            single(&note, "Created note", true)
                .unwrap()
                .contains(r#""Hi\u001b]0;pwned\u0007\nthere""#)
        );
    }

    #[test]
    fn the_caller_chooses_the_verb() {
        let note = Note {
            id: "note-1".into(),
            title: "Q4 planning".into(),
            folder: Some("Work".into()),
            modified: None,
            locked: false,
        };
        let reminder = Reminder {
            id: "reminder-1".into(),
            title: "Review pull request".into(),
            due: Some(date(2026, 10, 27)),
            priority: Some(Priority::High),
        };
        assert_snapshot!(
            single(&note, "Created note", false).unwrap(),
            @r#"Created note "Q4 planning" in Work"#
        );
        assert_snapshot!(
            single(&note, "Updated note", false).unwrap(),
            @r#"Updated note "Q4 planning" in Work"#
        );
        assert_snapshot!(
            single(&reminder, "Added", false).unwrap(),
            @r#"Added "Review pull request" (due 2026-10-27, high priority)"#
        );
        assert_snapshot!(
            single(&reminder, "Completed", false).unwrap(),
            @r#"Completed "Review pull request" (due 2026-10-27, high priority)"#
        );
    }
}

use comfy_table::{
    Attribute, Cell, ColumnConstraint, ContentArrangement, Row, Table, presets::NOTHING,
};
use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use miette::{IntoDiagnostic, Result};
use serde::{Serialize, Serializer};

use crate::app::doctor::{Check, CheckStatus};
use crate::domain::{
    ChangeCounts, Event, JournalSummary, Note, Priority, Reminder, time_zone_is_local_at,
};

pub trait Tabular: Serialize {
    const HEADER: &'static [&'static str];
    const TITLE_COLUMN: usize;
    fn row(&self, now: &Zoned) -> Vec<String>;
    fn cell_style(&self, column: usize, now: &Zoned) -> Option<CellStyle>;
}

pub trait Summary: Serialize {
    fn title(&self) -> &str;
    fn detail(&self) -> Option<String>;
    fn detail_at(&self, _now: Option<&Zoned>) -> Option<String> {
        self.detail()
    }
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

#[derive(Clone, Copy)]
pub enum CellStyle {
    Red,
    Yellow,
    Blue,
    Cyan,
    Dim,
}

pub fn list<T: Tabular>(items: &[T], json: bool, now: &Zoned, style: Style) -> Result<String> {
    if json {
        return to_json(items, style);
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
        let mut row = Row::from(item.row(now).iter().enumerate().map(|(column, text)| {
            let cell = Cell::new(clean(text));
            if style.color {
                marked_cell(cell, item.cell_style(column, now))
            } else {
                cell
            }
        }));
        row.max_height(1);
        table.add_row(row);
    }
    for (i, column) in table.column_iter_mut().enumerate() {
        column.set_padding((0, 2));
        if i != T::TITLE_COLUMN {
            column.set_constraint(ColumnConstraint::ContentWidth);
        }
    }
    Ok(resolve_color_markers(&table.trim_fmt()))
}

pub fn single<T: Summary>(item: &T, verb: &str, json: bool, style: Style) -> Result<String> {
    single_with_now(item, verb, json, style, None)
}

pub fn single_at<T: Summary>(
    item: &T,
    verb: &str,
    json: bool,
    style: Style,
    now: &Zoned,
) -> Result<String> {
    single_with_now(item, verb, json, style, Some(now))
}

fn single_with_now<T: Summary>(
    item: &T,
    verb: &str,
    json: bool,
    style: Style,
    now: Option<&Zoned>,
) -> Result<String> {
    if json {
        to_json(item, style)
    } else {
        let detail = item
            .detail_at(now)
            .map(|d| format!(" {d}"))
            .unwrap_or_default();
        Ok(clean(&format!("{verb} \"{}\"{detail}", item.title())))
    }
}

pub fn today(
    overdue: &[Reminder],
    due_today: &[Reminder],
    events: &[Event],
    now: &Zoned,
    json: bool,
    style: Style,
) -> Result<String> {
    if json {
        return to_json(
            &TodayJson {
                overdue,
                due_today,
                events,
            },
            style,
        );
    }
    let sections = [
        ("Overdue", list(overdue, false, now, style)?),
        ("Due today", list(due_today, false, now, style)?),
        ("Events", list(events, false, now, style)?),
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

pub fn journal(
    entries: &[JournalSummary],
    json: bool,
    now: &Zoned,
    style: Style,
) -> Result<String> {
    list(entries, json, now, style)
}

pub fn undo(
    id: &str,
    undo_id: &str,
    counts: ChangeCounts,
    json: bool,
    style: Style,
) -> Result<String> {
    if json {
        return to_json(
            &UndoJson {
                id,
                undo_id: Some(undo_id),
                applied: true,
                added: counts.added,
                updated: counts.updated,
                completed: counts.completed,
                deleted: counts.deleted,
            },
            style,
        );
    }
    Ok(format!(
        "Undid {} from {id}.",
        count(counts.total(), "change")
    ))
}

pub fn undo_cancelled(id: &str, json: bool, style: Style) -> Result<String> {
    if json {
        return to_json(
            &UndoJson {
                id,
                undo_id: None,
                applied: false,
                added: 0,
                updated: 0,
                completed: 0,
                deleted: 0,
            },
            style,
        );
    }
    Ok("Nothing was changed.".into())
}

pub fn undo_resolution(
    id: &str,
    applied: Option<bool>,
    json: bool,
    style: Style,
) -> Result<String> {
    if json {
        return to_json(
            &UndoResolutionJson {
                id,
                resolved: applied.is_some(),
                applied: applied == Some(true),
            },
            style,
        );
    }
    Ok(match applied {
        Some(true) => format!("Marked pending undo {id} as applied."),
        Some(false) => format!("Removed pending undo {id}."),
        None => format!("Nothing was changed. Undo {id} is still pending."),
    })
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

pub fn doctor(checks: &[Check], json: bool, style: Style) -> Result<String> {
    if json {
        let checks = checks.iter().map(DoctorCheckJson::from).collect::<Vec<_>>();
        return to_json(
            &DoctorJson {
                id: "doctor",
                ok: !checks.iter().any(|check| check.status == "fail"),
                checks,
            },
            style,
        );
    }
    Ok(checks
        .iter()
        .map(|check| {
            let symbol = match check.status {
                CheckStatus::Pass => styled_symbol("✓", "32", style),
                CheckStatus::Warning => styled_symbol("!", "33", style),
                CheckStatus::Fail => styled_symbol("✗", "31", style),
            };
            check.fix.as_ref().map_or_else(
                || format!("{symbol} {}", clean(&check.message)),
                |fix| format!("{symbol} {}\n  Fix: {}", clean(&check.message), clean(fix)),
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn styled_symbol(symbol: &str, color: &str, style: Style) -> String {
    if style.color {
        ansi(symbol, color)
    } else {
        symbol.into()
    }
}

fn styled_heading(heading: &str, style: Style) -> String {
    if style.color {
        ansi(heading, "1")
    } else {
        heading.into()
    }
}

// comfy-table renders named colors as 256-color codes (`38;5;N`), so cells are marked with
// attributes it renders as plain SGR codes, which `resolve_color_markers` swaps for 16-color codes.
fn marked_cell(cell: Cell, style: Option<CellStyle>) -> Cell {
    match style {
        Some(CellStyle::Red) => cell.add_attribute(Attribute::Underlined),
        Some(CellStyle::Yellow) => cell.add_attribute(Attribute::Italic),
        Some(CellStyle::Blue) => cell.add_attribute(Attribute::Reverse),
        Some(CellStyle::Cyan) => cell.add_attribute(Attribute::CrossedOut),
        Some(CellStyle::Dim) => cell.add_attribute(Attribute::Dim),
        None => cell,
    }
}

fn ansi(text: &str, code: &str) -> String {
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

fn resolve_color_markers(text: &str) -> String {
    text.replace("\u{1b}[4m", "\u{1b}[31m")
        .replace("\u{1b}[3m", "\u{1b}[33m")
        .replace("\u{1b}[7m", "\u{1b}[34m")
        .replace("\u{1b}[9m", "\u{1b}[36m")
}

fn to_json<T: Serialize + ?Sized>(value: &T, style: Style) -> Result<String> {
    let json = serde_json::to_string_pretty(value).into_diagnostic()?;
    if style.color {
        Ok(color_json(&json))
    } else {
        Ok(json)
    }
}

fn color_json(json: &str) -> String {
    let bytes = json.as_bytes();
    let mut colored = String::with_capacity(json.len());
    let mut start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let (end, color) = match bytes[index] {
            b'"' => {
                let end = json_string_end(bytes, index);
                let mut next = end;
                while bytes.get(next).is_some_and(u8::is_ascii_whitespace) {
                    next += 1;
                }
                (
                    end,
                    if bytes.get(next) == Some(&b':') {
                        "34"
                    } else {
                        "32"
                    },
                )
            }
            b'-' | b'0'..=b'9' => (json_number_end(bytes, index), "33"),
            b't' if bytes[index..].starts_with(b"true") => (index + 4, "33"),
            b'f' if bytes[index..].starts_with(b"false") => (index + 5, "33"),
            b'n' if bytes[index..].starts_with(b"null") => (index + 4, "2"),
            _ => {
                index += 1;
                continue;
            }
        };
        colored.push_str(&json[start..index]);
        colored.push_str("\u{1b}[");
        colored.push_str(color);
        colored.push('m');
        colored.push_str(&json[index..end]);
        colored.push_str("\u{1b}[0m");
        start = end;
        index = end;
    }
    colored.push_str(&json[start..]);
    colored
}

fn json_string_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn json_number_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E'))
    {
        index += 1;
    }
    index
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

    fn row(&self, _: &Zoned) -> Vec<String> {
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

    fn cell_style(&self, column: usize, _: &Zoned) -> Option<CellStyle> {
        match column {
            1 if self.folder.is_some() => Some(CellStyle::Cyan),
            2 if self.locked => Some(CellStyle::Yellow),
            _ => None,
        }
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

    fn row(&self, now: &Zoned) -> Vec<String> {
        vec![
            self.title.clone(),
            format_reminder_due(self, Some(now)),
            self.priority
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
        ]
    }

    fn cell_style(&self, column: usize, now: &Zoned) -> Option<CellStyle> {
        match column {
            1 if reminder_is_overdue(self, now) => Some(CellStyle::Red),
            1 if reminder_is_due_today(self, now.date(), now) => Some(CellStyle::Yellow),
            2 => match self.priority {
                Some(Priority::High) => Some(CellStyle::Red),
                Some(Priority::Medium) => Some(CellStyle::Yellow),
                Some(Priority::Low) => Some(CellStyle::Blue),
                None => None,
            },
            _ => None,
        }
    }
}

impl Summary for Reminder {
    fn title(&self) -> &str {
        &self.title
    }

    fn detail(&self) -> Option<String> {
        reminder_detail(self, None)
    }

    fn detail_at(&self, now: Option<&Zoned>) -> Option<String> {
        reminder_detail(self, now)
    }
}

fn reminder_detail(reminder: &Reminder, now: Option<&Zoned>) -> Option<String> {
    let details: Vec<String> = [
        reminder
            .due
            .map(|_| format!("due {}", format_reminder_due(reminder, now))),
        reminder
            .priority
            .map(|p| format!("{} priority", p.as_str())),
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

fn format_reminder_due(reminder: &Reminder, now: Option<&Zoned>) -> String {
    let Some(date) = reminder.due else {
        return String::new();
    };
    let Some(time) = reminder.due_time else {
        return date.to_string();
    };
    let mut due = format!("{date} {}", time.strftime("%H:%M"));
    if let Some(zone) = reminder.due_time_zone.as_deref() {
        let is_local = now.is_some_and(|now| {
            time_zone_is_local_at(zone, now.time_zone(), date, time).is_ok_and(|local| local)
        });
        if !is_local {
            due.push(' ');
            due.push_str(zone);
        }
    }
    due
}

pub(crate) fn reminder_is_overdue(reminder: &Reminder, now: &Zoned) -> bool {
    let Some(date) = reminder.due else {
        return false;
    };
    let Some(time) = reminder.due_time else {
        return date < now.date();
    };
    if let Some(due_at) = reminder.due_at {
        return due_at < now.timestamp();
    }
    let civil = date.at(time.hour(), time.minute(), 0, 0);
    civil < now.datetime()
}

pub(crate) fn reminder_is_due_today(reminder: &Reminder, date: Date, now: &Zoned) -> bool {
    if reminder.due_time.is_some()
        && let Some(due_at) = reminder.due_at
    {
        return due_at.to_zoned(now.time_zone().clone()).date() == date;
    }
    reminder.due == Some(date)
}

#[cfg(test)]
fn zoned_due_is_overdue(
    civil: jiff::civil::DateTime,
    zone: &jiff::tz::TimeZone,
    now: &Zoned,
) -> Option<bool> {
    civil
        .to_zoned(zone.clone())
        .ok()
        .map(|due| due.timestamp() < now.timestamp())
}

impl Tabular for Event {
    const HEADER: &'static [&'static str] = &["START", "TITLE"];
    const TITLE_COLUMN: usize = 1;

    fn row(&self, _: &Zoned) -> Vec<String> {
        let start = if self.all_day {
            self.start.strftime("%Y-%m-%d all day")
        } else {
            self.start.strftime("%Y-%m-%d %H:%M")
        };
        vec![start.to_string(), self.title.clone()]
    }

    fn cell_style(&self, column: usize, _: &Zoned) -> Option<CellStyle> {
        (column == 0).then_some(CellStyle::Dim)
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
    due_time: Option<String>,
    due_time_zone: Option<&'a str>,
    priority: Option<&'static str>,
}

impl Serialize for Reminder {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ReminderJson {
            id: &self.id,
            title: &self.title,
            due: self.due,
            due_time: self
                .due
                .and(self.due_time)
                .map(|time| time.strftime("%H:%M").to_string()),
            due_time_zone: self
                .due
                .and(self.due_time)
                .and(self.due_time_zone.as_deref()),
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

    fn row(&self, _: &Zoned) -> Vec<String> {
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

    fn cell_style(&self, _: usize, _: &Zoned) -> Option<CellStyle> {
        None
    }
}

#[derive(Serialize)]
struct DoctorJson {
    id: &'static str,
    ok: bool,
    checks: Vec<DoctorCheckJson>,
}

#[derive(Serialize)]
struct DoctorCheckJson {
    id: String,
    status: &'static str,
    message: String,
    fix: Option<String>,
}

impl From<&Check> for DoctorCheckJson {
    fn from(check: &Check) -> Self {
        Self {
            id: check.id.clone(),
            status: check.status.as_str(),
            message: check.message.clone(),
            fix: check.fix.clone(),
        }
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
    use jiff::tz::{TimeZone, offset};
    use proptest::prelude::*;
    use serde_json::Value;

    use super::*;

    fn test_today() -> Date {
        date(2026, 10, 26)
    }

    fn test_now() -> Zoned {
        test_today()
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    fn style(color: bool) -> Style {
        Style { color, width: None }
    }

    fn strip_ansi(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut plain = String::with_capacity(text.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index..].starts_with(b"\x1b[") {
                index += 2;
                while index < bytes.len() && bytes[index] != b'm' {
                    index += 1;
                }
                index += usize::from(index < bytes.len());
            } else {
                let character = text[index..].chars().next().unwrap();
                plain.push(character);
                index += character.len_utf8();
            }
        }
        plain
    }

    fn visible_escapes(text: &str) -> String {
        text.replace('\u{1b}', "\\x1b")
    }

    fn json_values() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|number| Value::Number(number.into())),
            any::<String>().prop_map(Value::String),
        ];
        leaf.prop_recursive(4, 64, 8, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::btree_map(any::<String>(), inner, 0..4)
                    .prop_map(|object| Value::Object(object.into_iter().collect())),
            ]
        })
    }

    fn reminders() -> Vec<Reminder> {
        vec![
            Reminder {
                id: "reminder-1".into(),
                title: "Book flights to Lisbon for the November offsite".into(),
                due: Some(date(2026, 10, 27)),
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: Some(Priority::High),
            },
            Reminder {
                id: "reminder-2".into(),
                title: "Buy groceries".into(),
                due: None,
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: None,
            },
        ]
    }

    #[test]
    fn rows_stay_whole_without_a_width() {
        assert_snapshot!(list(&reminders(), false, &test_now(), Style::default()).unwrap(), @r"
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
        assert_snapshot!(list(&reminders(), false, &test_now(), style).unwrap(), @r"
        TITLE             DUE         PRIORITY
        Book flights to…  2026-10-27  high
        Buy groceries
        ");
    }

    #[test]
    fn reminder_tables_show_times_and_only_foreign_zone_names() {
        let reminders = [
            Reminder {
                id: "local".into(),
                title: "Local call".into(),
                due: Some(test_today()),
                due_time: Some(jiff::civil::time(10, 0, 0, 0)),
                due_time_zone: Some("GMT-0400".into()),
                due_at: None,
                priority: None,
            },
            Reminder {
                id: "foreign".into(),
                title: "East coast call".into(),
                due: Some(test_today()),
                due_time: Some(jiff::civil::time(11, 0, 0, 0)),
                due_time_zone: Some("America/New_York".into()),
                due_at: None,
                priority: None,
            },
        ];
        let now = test_today()
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        assert_snapshot!(list(&reminders, false, &now, Style::default()).unwrap(), @r"
        TITLE            DUE                                PRIORITY
        Local call       2026-10-26 10:00
        East coast call  2026-10-26 11:00 America/New_York
        ");
    }

    #[test]
    fn offset_zone_visibility_uses_the_offset_at_the_due_time() {
        let reminder = |id: &str, zone: &str| Reminder {
            id: id.into(),
            title: id.into(),
            due: Some(date(2026, 12, 1)),
            due_time: Some(jiff::civil::time(9, 0, 0, 0)),
            due_time_zone: Some(zone.into()),
            due_at: None,
            priority: None,
        };
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap())
            .unwrap();
        let reminders = [
            reminder("local", "GMT-0600"),
            reminder("foreign", "GMT-0500"),
        ];
        assert_snapshot!(list(&reminders, false, &now, Style::default()).unwrap(), @r"
        TITLE    DUE                        PRIORITY
        local    2026-12-01 09:00
        foreign  2026-12-01 09:00 GMT-0500
        ");
        assert_eq!(
            single_at(&reminders[0], "Added", false, Style::default(), &now).unwrap(),
            "Added \"local\" (due 2026-12-01 09:00)"
        );
        assert_eq!(
            single_at(&reminders[1], "Added", false, Style::default(), &now).unwrap(),
            "Added \"foreign\" (due 2026-12-01 09:00 GMT-0500)"
        );
    }

    #[test]
    fn reminder_json_has_separate_time_and_zone_fields() {
        let reminder = Reminder {
            id: "timed".into(),
            title: "Call team".into(),
            due: Some(test_today()),
            due_time: Some(jiff::civil::time(9, 30, 0, 0)),
            due_time_zone: Some("America/New_York".into()),
            due_at: None,
            priority: None,
        };
        assert_snapshot!(single(&reminder, "Added", true, Style::default()).unwrap(), @r#"
        {
          "id": "timed",
          "title": "Call team",
          "due": "2026-10-26",
          "due_time": "09:30",
          "due_time_zone": "America/New_York",
          "priority": null
        }
        "#);
    }

    #[test]
    fn undated_reminder_json_hides_orphaned_time_fields() {
        let reminder = Reminder {
            id: "undated".into(),
            title: "Odd components".into(),
            due: None,
            due_time: Some(jiff::civil::time(9, 30, 0, 0)),
            due_time_zone: Some("America/New_York".into()),
            due_at: None,
            priority: None,
        };
        let value = serde_json::to_value(reminder).unwrap();
        assert_eq!(value["due_time"], Value::Null);
        assert_eq!(value["due_time_zone"], Value::Null);
    }

    #[test]
    fn overdue_comparison_has_defined_gap_and_fold_behavior() {
        let zone = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let gap = date(2027, 3, 14).at(2, 30, 0, 0);
        let before_gap_due = date(2027, 3, 14)
            .at(3, 15, 0, 0)
            .to_zoned(zone.clone())
            .unwrap();
        let after_gap_due = date(2027, 3, 14)
            .at(3, 45, 0, 0)
            .to_zoned(zone.clone())
            .unwrap();
        assert_eq!(
            zoned_due_is_overdue(gap, &zone, &before_gap_due),
            Some(false)
        );
        assert_eq!(zoned_due_is_overdue(gap, &zone, &after_gap_due), Some(true));

        let fold = date(2026, 11, 1).at(1, 30, 0, 0);
        let later_fold = zone.to_ambiguous_zoned(fold).later().unwrap();
        assert_eq!(zoned_due_is_overdue(fold, &zone, &later_fold), Some(true));
    }

    #[test]
    fn timed_today_and_overdue_checks_use_the_due_instant_in_the_local_zone() {
        let local = TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap();
        let now = date(2026, 10, 26).at(20, 0, 0, 0).to_zoned(local).unwrap();
        let tokyo_at = date(2026, 10, 27)
            .at(11, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(9)))
            .unwrap()
            .timestamp();
        let honolulu_at = date(2026, 10, 26)
            .at(23, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-10)))
            .unwrap()
            .timestamp();
        let tokyo = Reminder {
            id: "tokyo".into(),
            title: "Tokyo".into(),
            due: Some(date(2026, 10, 27)),
            due_time: Some(jiff::civil::time(11, 0, 0, 0)),
            due_time_zone: Some("Asia/Tokyo".into()),
            due_at: Some(tokyo_at),
            priority: None,
        };
        let honolulu = Reminder {
            id: "honolulu".into(),
            title: "Honolulu".into(),
            due: Some(date(2026, 10, 26)),
            due_time: Some(jiff::civil::time(23, 0, 0, 0)),
            due_time_zone: Some("Pacific/Honolulu".into()),
            due_at: Some(honolulu_at),
            priority: None,
        };
        assert!(reminder_is_due_today(&tokyo, now.date(), &now));
        assert!(!reminder_is_due_today(&honolulu, now.date(), &now));
        assert!(!reminder_is_overdue(&tokyo, &now));
        assert!(!reminder_is_overdue(&honolulu, &now));
    }

    #[test]
    fn colored_table_snapshot() {
        let reminders = vec![
            Reminder {
                id: "overdue".into(),
                title: "Submit expense report".into(),
                due: Some(date(2026, 10, 24)),
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: Some(Priority::High),
            },
            Reminder {
                id: "today".into(),
                title: "Review pull request".into(),
                due: Some(test_today()),
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: Some(Priority::Medium),
            },
            Reminder {
                id: "future".into(),
                title: "Plan team retreat".into(),
                due: Some(date(2026, 11, 2)),
                due_time: None,
                due_time_zone: None,
                due_at: None,
                priority: Some(Priority::Low),
            },
        ];
        let colored = list(&reminders, false, &test_now(), style(true)).unwrap();
        assert_snapshot!(visible_escapes(&colored), @r"
        \x1b[1mTITLE\x1b[0m                  \x1b[1mDUE\x1b[0m         \x1b[1mPRIORITY\x1b[0m
        Submit expense report  \x1b[31m2026-10-24\x1b[0m  \x1b[31mhigh\x1b[0m
        Review pull request    \x1b[33m2026-10-26\x1b[0m  \x1b[33mmedium\x1b[0m
        Plan team retreat      2026-11-02  \x1b[34mlow\x1b[0m
        ");
        assert!(!colored.contains("[38;5;"));
    }

    #[test]
    fn colored_table_alignment_ignores_escape_codes() {
        let colored = list(
            &reminders(),
            false,
            &test_now(),
            Style {
                color: true,
                width: Some(40),
            },
        )
        .unwrap();
        let plain = list(
            &reminders(),
            false,
            &test_now(),
            Style {
                color: false,
                width: Some(40),
            },
        )
        .unwrap();
        assert_eq!(strip_ansi(&colored), plain);
    }

    #[test]
    fn note_and_event_cells_use_their_semantic_styles() {
        let note = Note {
            id: "note-1".into(),
            title: "Launch plan".into(),
            folder: Some("Work".into()),
            modified: None,
            locked: true,
        };
        let note_table = list(&[note], false, &test_now(), style(true)).unwrap();
        assert!(note_table.contains("\u{1b}[36mWork\u{1b}[0m"));
        assert!(note_table.contains("\u{1b}[33myes\u{1b}[0m"));

        let event = Event {
            id: "event-1".into(),
            title: "Design review".into(),
            start: test_today()
                .at(14, 0, 0, 0)
                .to_zoned(TimeZone::fixed(offset(-4)))
                .unwrap(),
            all_day: false,
        };
        let event_table = list(&[event], false, &test_now(), style(true)).unwrap();
        assert!(event_table.contains("\u{1b}[2m2026-10-26 14:00\u{1b}[0m"));
        assert_eq!(
            styled_heading("Events", style(true)),
            "\u{1b}[1mEvents\u{1b}[0m"
        );
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
            single(&note, "Created note", false, Style::default()).unwrap(),
            @r#"Created note "Hi�]0;pwned� there""#
        );
        assert_snapshot!(list(std::slice::from_ref(&note), false, &test_now(), Style::default()).unwrap(), @r"
        TITLE               FOLDER  LOCKED
        Hi�]0;pwned� there
        ");
        assert!(
            single(&note, "Created note", true, Style::default())
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
            due_time: None,
            due_time_zone: None,
            due_at: None,
            priority: Some(Priority::High),
        };
        assert_snapshot!(
            single(&note, "Created note", false, Style::default()).unwrap(),
            @r#"Created note "Q4 planning" in Work"#
        );
        assert_snapshot!(
            single(&note, "Updated note", false, Style::default()).unwrap(),
            @r#"Updated note "Q4 planning" in Work"#
        );
        assert_snapshot!(
            single(&reminder, "Added", false, Style::default()).unwrap(),
            @r#"Added "Review pull request" (due 2026-10-27, high priority)"#
        );
        assert_snapshot!(
            single(&reminder, "Completed", false, Style::default()).unwrap(),
            @r#"Completed "Review pull request" (due 2026-10-27, high priority)"#
        );
    }

    #[test]
    fn colored_json_object_snapshot() {
        let note = Note {
            id: "note-1".into(),
            title: "Launch plan".into(),
            folder: Some("Work".into()),
            modified: None,
            locked: true,
        };
        let colored = single(&note, "Created note", true, style(true)).unwrap();
        assert_snapshot!(visible_escapes(&colored), @r#"
        {
          \x1b[34m"id"\x1b[0m: \x1b[32m"note-1"\x1b[0m,
          \x1b[34m"title"\x1b[0m: \x1b[32m"Launch plan"\x1b[0m,
          \x1b[34m"folder"\x1b[0m: \x1b[32m"Work"\x1b[0m,
          \x1b[34m"modified"\x1b[0m: \x1b[2mnull\x1b[0m,
          \x1b[34m"locked"\x1b[0m: \x1b[33mtrue\x1b[0m
        }
        "#);
        assert!(color_json(r#"{"count": -12.5e+3}"#).contains("\u{1b}[33m-12.5e+3\u{1b}[0m"));
    }

    proptest! {
        #[test]
        fn stripping_colored_json_gives_plain_json(value in json_values()) {
            let plain = to_json(&value, style(false)).unwrap();
            let colored = to_json(&value, style(true)).unwrap();
            prop_assert_eq!(strip_ansi(&colored), plain);
        }
    }
}

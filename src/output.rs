use comfy_table::{
    Attribute, Cell, ColumnConstraint, ContentArrangement, Row, Table, presets::NOTHING,
};
use jiff::Zoned;
use jiff::civil::Date;
use miette::{IntoDiagnostic, Result};
use serde::{Serialize, Serializer};

use crate::backend::{Event, Note, Reminder};

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

fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).into_diagnostic()
}

fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => '\u{fffd}',
            c => c,
        })
        .collect()
}

impl Tabular for Note {
    const HEADER: &'static [&'static str] = &["TITLE", "FOLDER"];
    const TITLE_COLUMN: usize = 0;

    fn row(&self) -> Vec<String> {
        vec![self.title.clone(), self.folder.clone().unwrap_or_default()]
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
        vec![
            self.start.strftime("%Y-%m-%d %H:%M").to_string(),
            self.title.clone(),
        ]
    }
}

#[derive(Serialize)]
struct NoteJson<'a> {
    id: &'a str,
    title: &'a str,
    folder: Option<&'a str>,
}

impl Serialize for Note {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        NoteJson {
            id: &self.id,
            title: &self.title,
            folder: self.folder.as_deref(),
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
            priority: self.priority.map(|p| p.as_str()),
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
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        EventJson {
            id: &self.id,
            title: &self.title,
            start: &self.start,
        }
        .serialize(serializer)
    }
}

fn rfc3339<S: Serializer>(time: &&Zoned, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&time.timestamp().display_with_offset(time.offset()))
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use jiff::civil::date;

    use super::*;
    use crate::backend::Priority;

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
        };
        assert_snapshot!(
            single(&note, "Created note", false).unwrap(),
            @r#"Created note "Hi�]0;pwned� there""#
        );
        assert_snapshot!(list(std::slice::from_ref(&note), false, Style::default()).unwrap(), @r"
        TITLE               FOLDER
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

use comfy_table::{
    Attribute, Cell, ColumnConstraint, ContentArrangement, Row, Table, presets::NOTHING,
};
use miette::{IntoDiagnostic, Result};
use serde::Serialize;

use crate::backend::{Event, Note, Reminder};

pub trait Tabular: Serialize {
    const HEADER: &'static [&'static str];
    const TITLE_COLUMN: usize;
    fn row(&self) -> Vec<String>;
}

pub trait Summary: Serialize {
    fn summary(&self) -> String;
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

pub fn single<T: Summary>(item: &T, json: bool) -> Result<String> {
    if json {
        to_json(item)
    } else {
        Ok(clean(&item.summary()))
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
    fn summary(&self) -> String {
        match &self.folder {
            Some(folder) => format!("Created note \"{}\" in {folder}", self.title),
            None => format!("Created note \"{}\"", self.title),
        }
    }
}

impl Tabular for Reminder {
    const HEADER: &'static [&'static str] = &["TITLE", "DUE", "PRIORITY"];
    const TITLE_COLUMN: usize = 0;

    fn row(&self) -> Vec<String> {
        vec![
            self.title.clone(),
            self.due.clone().unwrap_or_default(),
            self.priority
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
        ]
    }
}

impl Summary for Reminder {
    fn summary(&self) -> String {
        let details: Vec<String> = [
            self.due.as_ref().map(|d| format!("due {d}")),
            self.priority.map(|p| format!("{} priority", p.as_str())),
        ]
        .into_iter()
        .flatten()
        .collect();
        if details.is_empty() {
            format!("Added \"{}\"", self.title)
        } else {
            format!("Added \"{}\" ({})", self.title, details.join(", "))
        }
    }
}

impl Tabular for Event {
    const HEADER: &'static [&'static str] = &["START", "TITLE"];
    const TITLE_COLUMN: usize = 1;

    fn row(&self) -> Vec<String> {
        vec![self.start.clone(), self.title.clone()]
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;
    use crate::backend::Priority;

    fn reminders() -> Vec<Reminder> {
        vec![
            Reminder {
                title: "Book flights to Lisbon for the November offsite".into(),
                due: Some("2026-10-27".into()),
                priority: Some(Priority::High),
            },
            Reminder {
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
            title: "Hi\u{1b}]0;pwned\u{7}\nthere".into(),
            folder: None,
        };
        assert_snapshot!(single(&note, false).unwrap(), @r#"Created note "Hi�]0;pwned� there""#);
        assert_snapshot!(list(std::slice::from_ref(&note), false, Style::default()).unwrap(), @r"
        TITLE               FOLDER
        Hi�]0;pwned� there
        ");
        assert!(
            single(&note, true)
                .unwrap()
                .contains(r#""Hi\u001b]0;pwned\u0007\nthere""#)
        );
    }
}

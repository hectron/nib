use comfy_table::{Attribute, Cell, ContentArrangement, Table, presets::NOTHING};
use miette::{IntoDiagnostic, Result};
use serde::Serialize;

use crate::backend::{Event, Note, Reminder};

pub trait Tabular: Serialize {
    const HEADER: &'static [&'static str];
    fn row(&self) -> Vec<String>;
}

pub trait Summary: Serialize {
    fn summary(&self) -> String;
}

pub fn list<T: Tabular>(items: &[T], json: bool) -> Result<String> {
    if json {
        return to_json(items);
    }
    if items.is_empty() {
        return Ok(String::new());
    }
    let mut table = Table::new();
    table
        .load_style(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(
            T::HEADER
                .iter()
                .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
        )
        .add_rows(items.iter().map(Tabular::row));
    for column in table.column_iter_mut() {
        column.set_padding((0, 2));
    }
    Ok(table.trim_fmt())
}

pub fn single<T: Summary>(item: &T, json: bool) -> Result<String> {
    if json {
        to_json(item)
    } else {
        Ok(item.summary())
    }
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).into_diagnostic()
}

impl Tabular for Note {
    const HEADER: &'static [&'static str] = &["TITLE", "FOLDER"];

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

    fn row(&self) -> Vec<String> {
        vec![self.start.clone(), self.title.clone()]
    }
}

use std::collections::HashMap;
use std::fmt::Write as _;

use jiff::civil::Date;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::domain::{
    FieldEdit, Priority, ReminderChange, ReminderEdit, ReminderSnapshot, StoredReminder,
};

const HELP: &str = "<!-- nib task edit
Edit titles, due dates (📅 YYYY-MM-DD), and priority (⏫ high, 🔼 medium, 🔽 low).
Check a box ([x]) to complete a reminder. Add a line without a 🆔 to create one.
Delete a line to delete that reminder. Move a line under another heading to move it.
Indented lines are notes. Save and quit to review the changes before anything is applied.
-->";
const PRETTIER_START: &str = "<!-- prettier-ignore-start -->";
const PRETTIER_END: &str = "<!-- prettier-ignore-end -->";
const DUE: &str = "📅";
const ID: &str = "🆔";
const NOTE_INDENT: &str = "    ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub line: usize,
    pub id: String,
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub line: usize,
    pub id: Option<String>,
    pub done: bool,
    pub title: String,
    pub priority: Option<Priority>,
    pub due: Option<Date>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

fn error(line: usize, message: impl Into<String>) -> ParseError {
    ParseError {
        line,
        message: message.into(),
    }
}

pub struct Session {
    pub document: Document,
    pub skipped: usize,
    reminders: HashMap<String, usize>,
    lists: HashMap<String, usize>,
}

impl Session {
    pub fn new(snapshot: &ReminderSnapshot) -> Self {
        let list_ids = short_ids(snapshot.lists.iter().map(|l| l.id.as_str()));
        let reminder_ids = short_ids(snapshot.reminders.iter().map(|r| r.id.as_str()));
        let mut sections = Vec::new();
        let mut reminders = HashMap::new();
        for (list, short) in snapshot.lists.iter().zip(&list_ids) {
            let mut items = Vec::new();
            for (index, reminder) in snapshot.reminders.iter().enumerate() {
                if reminder.list_id != list.id || !representable(reminder) {
                    continue;
                }
                reminders.insert(reminder_ids[index].clone(), index);
                items.push(Item {
                    line: 0,
                    id: Some(reminder_ids[index].clone()),
                    done: false,
                    title: normalize_title(&reminder.title),
                    priority: reminder.priority,
                    due: reminder.due,
                    notes: normalize_notes(reminder.notes.as_deref()),
                });
            }
            sections.push(Section {
                line: 0,
                id: short.clone(),
                name: single_line(&list.name),
                items,
            });
        }
        let lists = list_ids
            .into_iter()
            .enumerate()
            .map(|(i, s)| (s, i))
            .collect();
        Session {
            skipped: snapshot.reminders.len() - reminders.len(),
            document: Document { sections },
            reminders,
            lists,
        }
    }
}

pub fn render(document: &Document) -> String {
    let blocks: Vec<String> = document
        .sections
        .iter()
        .map(|section| {
            let mut block = format!("## {} {ID} {}", escape(&section.name, false), section.id);
            if !section.items.is_empty() {
                block.push('\n');
                for item in &section.items {
                    block.push('\n');
                    block.push_str(&render_item(item));
                }
            }
            block
        })
        .collect();
    let mut out = format!("{HELP}\n{PRETTIER_START}\n\n");
    if !blocks.is_empty() {
        out.push_str(&blocks.join("\n\n"));
        out.push_str("\n\n");
    }
    out.push_str(PRETTIER_END);
    out.push('\n');
    out
}

fn render_item(item: &Item) -> String {
    let mut line = format!("- [{}]", if item.done { 'x' } else { ' ' });
    if !item.title.is_empty() {
        line.push(' ');
        line.push_str(&escape(&item.title, true));
    }
    if let Some(priority) = item.priority {
        line.push(' ');
        line.push_str(priority_marker(priority));
    }
    if let Some(due) = item.due {
        write!(&mut line, " {DUE} {due}").expect("writing to a String cannot fail");
    }
    if let Some(id) = &item.id {
        write!(&mut line, " {ID} {id}").expect("writing to a String cannot fail");
    }
    for note in item.notes.iter().flat_map(|n| n.split('\n')) {
        line.push('\n');
        if !note.is_empty() {
            line.push_str(NOTE_INDENT);
            line.push_str(note);
        }
    }
    line
}

pub fn parse(text: &str) -> Result<Document, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut sections: Vec<Section> = Vec::new();
    let mut comment_start = None;
    let mut collecting_notes = false;
    let mut blank_lines = 0;
    for (index, raw) in text.split('\n').enumerate() {
        let number = index + 1;
        let line = raw.trim_end();
        if comment_start.is_some() {
            if line.ends_with("-->") {
                comment_start = None;
            }
            continue;
        }
        if line.is_empty() {
            blank_lines += 1;
            continue;
        }
        if collecting_notes && let Some(note) = note_content(line) {
            let item = sections
                .last_mut()
                .and_then(|s| s.items.last_mut())
                .expect("notes follow an item");
            let notes = item.notes.get_or_insert_with(String::new);
            if !notes.is_empty() {
                notes.push_str(&"\n".repeat(blank_lines + 1));
            }
            notes.push_str(note);
            blank_lines = 0;
            continue;
        }
        collecting_notes = false;
        blank_lines = 0;
        if line.starts_with("<!--") {
            if !line.ends_with("-->") {
                comment_start = Some(number);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            let tail = read_tail(rest, false).map_err(|m| error(number, m))?;
            let id = tail.id.ok_or_else(|| {
                error(
                    number,
                    "list headings need their 🆔; nib can't create lists, so move reminders under an existing heading",
                )
            })?;
            sections.push(Section {
                line: number,
                id,
                name: tail.title,
                items: Vec::new(),
            });
        } else if line.starts_with('#') {
            return Err(error(
                number,
                "only `## <list> 🆔 <id>` headings are allowed",
            ));
        } else if let Some((done, rest)) = item_line(line) {
            let tail = read_tail(rest, true).map_err(|m| error(number, m))?;
            let section = sections.last_mut().ok_or_else(|| {
                error(number, "this reminder comes before the first list heading")
            })?;
            section.items.push(Item {
                line: number,
                id: tail.id,
                done,
                title: tail.title,
                priority: tail.priority,
                due: tail.due,
                notes: None,
            });
            collecting_notes = true;
        } else if note_content(line).is_some() {
            return Err(error(number, "notes must come right after a reminder"));
        } else if line.starts_with(' ') {
            return Err(error(number, "indent notes with 4 spaces or a tab"));
        } else if line.starts_with("- [-]") {
            return Err(error(
                number,
                "nib doesn't support [-]; delete the line to delete a reminder",
            ));
        } else {
            return Err(error(
                number,
                "expected a reminder (`- [ ] title`), a list heading, an indented note, or a comment",
            ));
        }
    }
    if let Some(start) = comment_start {
        return Err(error(start, "this comment is never closed with -->"));
    }
    Ok(Document { sections })
}

fn item_line(line: &str) -> Option<(bool, &str)> {
    let rest = line.strip_prefix("- [")?;
    let mut chars = rest.chars();
    let done = match chars.next()? {
        ' ' => false,
        'x' | 'X' => true,
        _ => return None,
    };
    let rest = chars.as_str().strip_prefix(']')?;
    if rest.is_empty() {
        return Some((done, rest));
    }
    let first = rest.chars().next()?;
    first
        .is_whitespace()
        .then(|| (done, &rest[first.len_utf8()..]))
}

fn note_content(line: &str) -> Option<&str> {
    line.strip_prefix(NOTE_INDENT)
        .or_else(|| line.strip_prefix('\t'))
}

struct Tail {
    title: String,
    priority: Option<Priority>,
    due: Option<Date>,
    id: Option<String>,
}

fn read_tail(rest: &str, item: bool) -> Result<Tail, String> {
    let mut text = rest;
    let mut tail = Tail {
        title: String::new(),
        priority: None,
        due: None,
        id: None,
    };
    while let Some((token, before)) = take_token(text, item)? {
        let duplicate = match token {
            Token::Priority(p) => tail.priority.replace(p).is_some(),
            Token::Due(d) => tail.due.replace(d).is_some(),
            Token::Id(id) => tail.id.replace(id).is_some(),
        };
        if duplicate {
            return Err("this line has the same field twice".into());
        }
        text = before;
    }
    tail.title = unescape(text.trim(), item);
    Ok(tail)
}

enum Token {
    Priority(Priority),
    Due(Date),
    Id(String),
}

fn take_token(text: &str, item: bool) -> Result<Option<(Token, &str)>, String> {
    let Some((before, word)) = last_word(text) else {
        return Ok(None);
    };
    if item && let Some(priority) = read_priority(word.trim_start_matches('\\')) {
        if word.starts_with('\\') {
            return Ok(None);
        }
        return Ok(Some((Token::Priority(priority), before)));
    }
    let Some((before_marker, marker)) = last_word(before) else {
        return Ok(None);
    };
    match strip_variation(marker) {
        DUE if item => match parse_date(word) {
            Some(date) => Ok(Some((Token::Due(date), before_marker))),
            None => Err(format!("\"{word}\" isn't a due date; use YYYY-MM-DD")),
        },
        ID if valid_id(word) => Ok(Some((Token::Id(word.to_lowercase()), before_marker))),
        ID => Err(format!("\"{word}\" isn't a valid 🆔")),
        _ => Ok(None),
    }
}

fn tail_marker(text: &str, item: bool) -> Option<(usize, usize)> {
    let (before, word) = last_word(text)?;
    let bare = word.trim_start_matches('\\');
    if item && read_priority(bare).is_some() {
        return Some((offset(text, word), word.len() - bare.len()));
    }
    let (_, marker) = last_word(before)?;
    let bare = marker.trim_start_matches('\\');
    match strip_variation(bare) {
        DUE if item => Some((offset(text, marker), marker.len() - bare.len())),
        ID => Some((offset(text, marker), marker.len() - bare.len())),
        _ => None,
    }
}

fn escape(text: &str, item: bool) -> String {
    match tail_marker(text, item) {
        Some((at, _)) => format!("{}\\{}", &text[..at], &text[at..]),
        None => text.to_string(),
    }
}

fn unescape(text: &str, item: bool) -> String {
    match tail_marker(text, item) {
        Some((at, slashes)) if slashes > 0 => format!("{}{}", &text[..at], &text[at + 1..]),
        _ => text.to_string(),
    }
}

fn last_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    match text.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
        Some((at, c)) => Some((text[..at].trim_end(), &text[at + c.len_utf8()..])),
        None => Some(("", text)),
    }
}

fn offset(outer: &str, inner: &str) -> usize {
    inner.as_ptr() as usize - outer.as_ptr() as usize
}

fn strip_variation(word: &str) -> &str {
    word.strip_suffix(['\u{fe0f}', '\u{fe0e}']).unwrap_or(word)
}

fn read_priority(word: &str) -> Option<Priority> {
    match strip_variation(word) {
        "⏫" | "🔺" => Some(Priority::High),
        "🔼" => Some(Priority::Medium),
        "🔽" | "⏬" => Some(Priority::Low),
        _ => None,
    }
}

fn priority_marker(priority: Priority) -> &'static str {
    match priority {
        Priority::High => "⏫",
        Priority::Medium => "🔼",
        Priority::Low => "🔽",
    }
}

fn parse_date(word: &str) -> Option<Date> {
    let shaped = word.len() == 10
        && word.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    shaped.then(|| word.parse().ok()).flatten()
}

fn valid_id(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn short_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let ids: Vec<String> = ids
        .map(|id| {
            id.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_lowercase()
        })
        .collect();
    let longest = ids.iter().map(String::len).max().unwrap_or(0);
    let mut length = 8.min(longest);
    loop {
        let mut prefixes: Vec<&str> = ids.iter().map(|id| &id[..length.min(id.len())]).collect();
        prefixes.sort_unstable();
        prefixes.dedup();
        if prefixes.len() == ids.len() || length >= longest {
            return ids
                .iter()
                .map(|id| id[..length.min(id.len())].to_string())
                .collect();
        }
        length += 1;
    }
}

pub fn normalize_title(title: &str) -> String {
    title.trim().to_string()
}

pub fn normalize_notes(notes: Option<&str>) -> Option<String> {
    let notes = notes?.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = notes.split('\n').map(str::trim_end).collect();
    let first = lines.iter().position(|l| !l.is_empty())?;
    let last = lines.iter().rposition(|l| !l.is_empty())?;
    Some(lines[first..=last].join("\n"))
}

fn comparable(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let body = line.trim_start_matches('\t');
            format!("{}{}", NOTE_INDENT.repeat(line.len() - body.len()), body)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .nfc()
        .collect()
}

fn same(a: &str, b: &str) -> bool {
    comparable(a) == comparable(b)
}

fn same_notes(a: Option<&str>, b: Option<&str>) -> bool {
    match (normalize_notes(a), normalize_notes(b)) {
        (Some(a), Some(b)) => same(&a, &b),
        (None, None) => true,
        _ => false,
    }
}

fn representable(reminder: &StoredReminder) -> bool {
    !reminder.title.contains(['\n', '\r'])
        && representable_text(&reminder.title)
        && reminder.notes.as_deref().is_none_or(representable_text)
}

fn representable_text(text: &str) -> bool {
    text.chars().all(|c| match c {
        '\t' | '\n' | '\r' => true,
        '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {
            false
        }
        c => !c.is_control(),
    })
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub changes: Vec<ReminderChange>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn deletions(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| matches!(c, ReminderChange::Delete { .. }))
            .count()
    }
}

fn validate_document(
    snapshot: &ReminderSnapshot,
    session: &Session,
    edited: &Document,
) -> Result<(), ParseError> {
    let item_count: usize = edited
        .sections
        .iter()
        .map(|section| section.items.len())
        .sum();
    if item_count == 0 && !session.reminders.is_empty() {
        return Err(error(
            1,
            "the file has no reminders left, so nothing was changed; delete reminders a few at a time",
        ));
    }
    let mut seen_lists = HashMap::new();
    for section in &edited.sections {
        if !session.lists.contains_key(&section.id) {
            return Err(error(
                section.line,
                format!("no list has the 🆔 {}", section.id),
            ));
        }
        if let Some(other) = seen_lists.insert(&section.id, section.line) {
            return Err(error(
                section.line,
                format!("this list already has a heading on line {other}"),
            ));
        }
    }
    let mut missing: Vec<_> = session
        .lists
        .iter()
        .filter(|(id, _)| !seen_lists.contains_key(id))
        .map(|(id, index)| (&snapshot.lists[*index].name, id))
        .collect();
    missing.sort_unstable();
    if let Some((name, id)) = missing.first() {
        return Err(error(
            1,
            format!(
                "the original list heading \"{name}\" 🆔 {id} is missing; restore it before changing reminders"
            ),
        ));
    }
    Ok(())
}

fn plan_deletions(
    snapshot: &ReminderSnapshot,
    session: &Session,
    seen_items: &HashMap<String, usize>,
    adds: &[(usize, String)],
    plan: &mut Plan,
) -> Result<(), ParseError> {
    let mut missing: Vec<&usize> = session
        .reminders
        .iter()
        .filter(|(short, _)| !seen_items.contains_key(*short))
        .map(|(_, index)| index)
        .collect();
    missing.sort_unstable();
    for &index in missing {
        let before = &snapshot.reminders[index];
        if snapshot
            .lists
            .iter()
            .find(|list| list.id == before.list_id)
            .is_some_and(|list| !list.writable)
        {
            return Err(error(
                1,
                format!(
                    "\"{}\" is in a read-only list and can't be deleted",
                    before.title
                ),
            ));
        }
        let title = normalize_title(&before.title);
        if let Some((line, _)) = adds
            .iter()
            .find(|(_, added)| same(&added.to_lowercase(), &title.to_lowercase()))
        {
            plan.warnings.push(format!(
                "line {line}: \"{title}\" lost its 🆔, so nib will delete it and add a new one, dropping alarms, repeats, and links"
            ));
        }
        plan.changes.push(ReminderChange::Delete {
            before: before.clone(),
        });
    }
    Ok(())
}

pub fn plan(
    snapshot: &ReminderSnapshot,
    session: &Session,
    edited: &Document,
) -> Result<Plan, ParseError> {
    validate_document(snapshot, session, edited)?;
    let mut plan = Plan::default();
    let mut seen_items: HashMap<String, usize> = HashMap::new();
    let mut adds: Vec<(usize, String)> = Vec::new();
    for section in &edited.sections {
        let list_index = *session
            .lists
            .get(&section.id)
            .ok_or_else(|| error(section.line, format!("no list has the 🆔 {}", section.id)))?;
        let list = &snapshot.lists[list_index];
        if !same(&section.name, &single_line(&list.name)) {
            plan.warnings.push(format!(
                "line {}: renaming lists isn't supported, so \"{}\" keeps its name",
                section.line, list.name
            ));
        }
        for item in &section.items {
            let Some(short) = &item.id else {
                if item.title.is_empty() {
                    return Err(error(item.line, "a new reminder needs a title"));
                }
                if item.done {
                    return Err(error(item.line, "a new reminder can't start out completed"));
                }
                if !list.writable {
                    return Err(error(item.line, format!("\"{}\" is read-only", list.name)));
                }
                adds.push((item.line, item.title.clone()));
                plan.changes.push(ReminderChange::Add {
                    list_id: list.id.clone(),
                    title: item.title.clone(),
                    notes: item.notes.clone(),
                    due: item.due,
                    priority: item.priority,
                });
                continue;
            };
            let index = *session.reminders.get(short).ok_or_else(|| {
                error(
                    item.line,
                    format!("no reminder in this file has the 🆔 {short}"),
                )
            })?;
            if let Some(other) = seen_items.insert(short.clone(), item.line) {
                return Err(error(
                    item.line,
                    format!(
                        "the 🆔 {short} is also on line {other}; remove it from the copy to add a new reminder"
                    ),
                ));
            }
            let before = &snapshot.reminders[index];
            let edit = ReminderEdit {
                title: (!same(&item.title, &normalize_title(&before.title)))
                    .then(|| item.title.clone()),
                notes: if same_notes(item.notes.as_deref(), before.notes.as_deref()) {
                    FieldEdit::Unchanged
                } else {
                    item.notes.clone().map_or(FieldEdit::Remove, FieldEdit::Set)
                },
                due: field_edit(item.due, before.due),
                priority: field_edit(item.priority, before.priority),
                list_id: (list.id != before.list_id).then(|| list.id.clone()),
                complete: item.done,
            };
            if edit == ReminderEdit::default() {
                continue;
            }
            let source = snapshot.lists.iter().find(|l| l.id == before.list_id);
            if source.is_some_and(|l| !l.writable) || (edit.list_id.is_some() && !list.writable) {
                return Err(error(item.line, "that change touches a read-only list"));
            }
            if edit.due.is_changed() && before.alarms_present {
                plan.warnings.push(format!(
                    "line {}: \"{}\" has an alarm set for a specific time, which stays where it is",
                    item.line, item.title
                ));
            }
            plan.changes.push(ReminderChange::Update {
                before: before.clone(),
                edit,
            });
        }
    }
    plan_deletions(snapshot, session, &seen_items, &adds, &mut plan)?;
    Ok(plan)
}

pub fn describe(plan: &Plan, snapshot: &ReminderSnapshot) -> String {
    let list_name = |id: &str| {
        snapshot
            .lists
            .iter()
            .find(|l| l.id == id)
            .map_or_else(|| id.to_string(), |l| l.name.clone())
    };
    let mut lines = Vec::new();
    for change in &plan.changes {
        lines.push(match change {
            ReminderChange::Add { list_id, title, .. } => {
                format!("  + add       \"{title}\" to {}", list_name(list_id))
            }
            ReminderChange::Delete { before } => {
                let losses: Vec<&str> = [
                    (before.alarms_present, "alarms"),
                    (before.recurrence_present, "repeats"),
                    (before.url.is_some(), "a link"),
                ]
                .into_iter()
                .filter_map(|(has, what)| has.then_some(what))
                .collect();
                let mut line = format!("  - delete    \"{}\"", normalize_title(&before.title));
                if !losses.is_empty() {
                    write!(&mut line, " (loses {})", losses.join(", "))
                        .expect("writing to a String cannot fail");
                }
                line
            }
            ReminderChange::Update { before, edit } => {
                let title = normalize_title(&before.title);
                let mut parts = Vec::new();
                if let Some(new) = &edit.title {
                    parts.push(format!("title → \"{new}\""));
                }
                match (&edit.due, before.due) {
                    (FieldEdit::Set(new), Some(old)) => parts.push(format!("due {old} → {new}")),
                    (FieldEdit::Set(new), None) => parts.push(format!("due {new}")),
                    (FieldEdit::Remove, _) => parts.push("due date removed".into()),
                    (FieldEdit::Unchanged, _) => {}
                }
                match edit.priority {
                    FieldEdit::Set(priority) => {
                        parts.push(format!("priority {}", priority.as_str()));
                    }
                    FieldEdit::Remove => parts.push("priority removed".into()),
                    FieldEdit::Unchanged => {}
                }
                if edit.notes.is_changed() {
                    parts.push("notes".into());
                }
                if let Some(list) = &edit.list_id {
                    parts.push(format!("moved to {}", list_name(list)));
                }
                if edit.complete && parts.is_empty() {
                    format!("  ✓ complete  \"{title}\"")
                } else {
                    if edit.complete {
                        parts.push("completed".into());
                    }
                    format!("  ~ update    \"{title}\": {}", parts.join(", "))
                }
            }
        });
    }
    for warning in &plan.warnings {
        lines.push(format!("  ! {warning}"));
    }
    lines.join("\n")
}

fn field_edit<T: Copy + Eq>(after: Option<T>, before: Option<T>) -> FieldEdit<T> {
    if after == before {
        FieldEdit::Unchanged
    } else {
        after.map_or(FieldEdit::Remove, FieldEdit::Set)
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use jiff::civil::date;
    use proptest::prelude::*;

    use super::*;
    use crate::domain::ReminderList;

    const REMINDERS: &str = "AAAA1111-0000-0000-0000-000000000001";
    const GROCERIES: &str = "BBBB2222-0000-0000-0000-000000000002";
    const SOMEDAY: &str = "CCCC3333-0000-0000-0000-000000000003";

    fn list(id: &str, name: &str) -> ReminderList {
        ReminderList {
            id: id.into(),
            name: name.into(),
            writable: true,
        }
    }

    fn reminder(id: &str, list_id: &str, title: &str) -> StoredReminder {
        StoredReminder {
            id: id.into(),
            list_id: list_id.into(),
            title: title.into(),
            notes: None,
            due: None,
            due_components: None,
            priority: None,
            modified: None,
            alarms_present: false,
            recurrence_present: false,
            url: None,
            backup_partial: false,
        }
    }

    fn snapshot() -> ReminderSnapshot {
        ReminderSnapshot {
            lists: vec![
                list(REMINDERS, "Reminders"),
                list(GROCERIES, "Groceries"),
                list(SOMEDAY, "Someday"),
            ],
            reminders: vec![
                StoredReminder {
                    priority: Some(Priority::High),
                    due: Some(date(2026, 10, 27)),
                    ..reminder(
                        "E37CE785-1111-2222-3333-444455556666",
                        REMINDERS,
                        "Review pull request",
                    )
                },
                StoredReminder {
                    priority: Some(Priority::Medium),
                    due: Some(date(2026, 10, 29)),
                    notes: Some("Bring the checkbook\n\nSecond paragraph".into()),
                    ..reminder(
                        "4B19A0C2-1111-2222-3333-444455556666",
                        REMINDERS,
                        "Pay rent",
                    )
                },
                reminder(
                    "9C02D1AA-1111-2222-3333-444455556666",
                    GROCERIES,
                    "Oat milk",
                ),
            ],
        }
    }

    fn without_lines(mut document: Document) -> Document {
        for section in &mut document.sections {
            section.line = 0;
            for item in &mut section.items {
                item.line = 0;
            }
        }
        document
    }

    fn plan_after(change: impl Fn(String) -> String) -> Result<Plan, ParseError> {
        let snapshot = snapshot();
        let session = Session::new(&snapshot);
        let edited = change(render(&session.document));
        plan(&snapshot, &session, &parse(&edited)?)
    }

    fn replace(text: impl AsRef<str>, from: &str, to: &str) -> String {
        let text = text.as_ref();
        assert!(text.contains(from), "{from:?} not in:\n{text}");
        text.replacen(from, to, 1)
    }

    fn only_update(plan: Plan) -> ReminderEdit {
        assert_eq!(plan.changes.len(), 1, "{plan:?}");
        match plan.changes.into_iter().next() {
            Some(ReminderChange::Update { edit, .. }) => edit,
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn renders_lists_reminders_and_notes() {
        assert_snapshot!(render(&Session::new(&snapshot()).document), @r"
        <!-- nib task edit
        Edit titles, due dates (📅 YYYY-MM-DD), and priority (⏫ high, 🔼 medium, 🔽 low).
        Check a box ([x]) to complete a reminder. Add a line without a 🆔 to create one.
        Delete a line to delete that reminder. Move a line under another heading to move it.
        Indented lines are notes. Save and quit to review the changes before anything is applied.
        -->
        <!-- prettier-ignore-start -->

        ## Reminders 🆔 aaaa1111

        - [ ] Review pull request ⏫ 📅 2026-10-27 🆔 e37ce785
        - [ ] Pay rent 🔼 📅 2026-10-29 🆔 4b19a0c2
            Bring the checkbook

            Second paragraph

        ## Groceries 🆔 bbbb2222

        - [ ] Oat milk 🆔 9c02d1aa

        ## Someday 🆔 cccc3333

        <!-- prettier-ignore-end -->
        ");
    }

    #[test]
    fn rendered_file_parses_back_exactly() {
        let session = Session::new(&snapshot());
        let text = render(&session.document);
        let parsed = parse(&text).unwrap();
        assert_eq!(without_lines(parsed.clone()), session.document);
        assert_eq!(render(&parsed), text);
    }

    #[test]
    fn titles_are_verbatim_except_one_escape_before_a_trailing_marker() {
        let cases = [
            ("*bold* #tag [link] 1. done", "*bold* #tag [link] 1. done"),
            ("Read \\ paper", "Read \\ paper"),
            ("Pay 📅", "Pay 📅"),
            ("Keep 📅 2026-01-01", "Keep \\📅 2026-01-01"),
            ("Keep \\📅 2026-01-01", "Keep \\\\📅 2026-01-01"),
            ("Report 🔼 📅 2026-01-01", "Report 🔼 \\📅 2026-01-01"),
            ("Plan 📅 someday", "Plan \\📅 someday"),
            ("Urgent ⏫", "Urgent \\⏫"),
            ("Urgent ⏫\u{fe0f}", "Urgent \\⏫\u{fe0f}"),
            ("Due 📅 \\⏫", "Due 📅 \\\\⏫"),
            ("Use the 🆔 badge", "Use the \\🆔 badge"),
            ("📅 2026-01-01", "\\📅 2026-01-01"),
        ];
        for (title, rendered) in cases {
            assert_eq!(escape(title, true), rendered, "{title}");
            let text = format!("## L 🆔 l1\n- [ ] {rendered} ⏫ 🆔 abc\n");
            let item = &parse(&text).unwrap().sections[0].items[0];
            assert_eq!(item.title, title);
            assert_eq!(item.priority, Some(Priority::High), "{title}");
            assert_eq!(item.id.as_deref(), Some("abc"), "{title}");
        }
    }

    #[test]
    fn markers_parse_in_any_order_with_variants() {
        let text = "## L 🆔 l1\n- [X] Pay rent 🆔 ABC 📅\u{fe0f} 2026-10-29\u{3000}🔺\n";
        let item = &parse(text).unwrap().sections[0].items[0];
        assert!(item.done);
        assert_eq!(item.title, "Pay rent");
        assert_eq!(item.id.as_deref(), Some("abc"));
        assert_eq!(item.due, Some(date(2026, 10, 29)));
        assert_eq!(item.priority, Some(Priority::High));
    }

    #[test]
    fn every_line_must_mean_something() {
        let cases = [
            ("- [ ] Pay 📅 2026-02-30", "isn't a due date"),
            ("- [ ] Pay 📅 2026-1-5", "isn't a due date"),
            ("- [ ] Pay 🆔 bad_id", "isn't a valid 🆔"),
            ("- [ ] Pay ⏫ 🔽", "same field twice"),
            ("- [-] Pay", "doesn't support [-]"),
            ("* [ ] Pay", "expected a reminder"),
            ("- [] Pay", "expected a reminder"),
            ("- [ ]Pay", "expected a reminder"),
            ("  - [ ] Pay", "indent notes with 4 spaces"),
            ("# Title", "only `## <list> 🆔 <id>` headings"),
            ("## New list", "need their 🆔"),
            ("<!-- never closed", "never closed"),
            ("stray text", "expected a reminder"),
        ];
        for (line, message) in cases {
            let err = parse(&format!("## L 🆔 l1\n{line}\n")).unwrap_err();
            assert_eq!(err.line, 2, "{line}");
            assert!(err.message.contains(message), "{line}: {}", err.message);
        }
        let err = parse("- [ ] Pay\n").unwrap_err();
        assert!(err.message.contains("before the first list heading"));
        let err = parse("## L 🆔 l1\n    stray note\n").unwrap_err();
        assert!(err.message.contains("right after a reminder"));
    }

    #[test]
    fn empty_titles_survive_editors_stripping_the_space() {
        let item = &parse("## L 🆔 l1\n- [ ]\n").unwrap().sections[0].items[0];
        assert_eq!(item.title, "");
    }

    #[test]
    fn what_editors_do_to_files_is_not_a_change() {
        let snapshot = snapshot();
        let session = Session::new(&snapshot);
        let text = render(&session.document);
        let rewrites = [
            ("no final newline", text.trim_end().to_string()),
            ("two final newlines", format!("{text}\n")),
            ("windows line endings", text.replace('\n', "\r\n")),
            ("byte order mark", format!("\u{feff}{text}")),
            ("tab-indented notes", text.replace("\n    ", "\n\t")),
            ("extra blank lines", text.replace("\n- [ ]", "\n\n- [ ]")),
            (
                "trailing spaces",
                text.replace("2026-10-27", "2026-10-27   "),
            ),
            ("ideographic spaces", text.replace(" 🆔", "\u{3000}🆔")),
            (
                "emoji variation selectors",
                text.replace("⏫", "⏫\u{fe0f}"),
            ),
        ];
        for (what, edited) in rewrites {
            let document = parse(&edited).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(
                plan(&snapshot, &session, &document).unwrap(),
                Plan::default(),
                "{what}"
            );
        }
    }

    #[test]
    fn apple_side_whitespace_and_unicode_forms_are_not_changes() {
        let snapshot = ReminderSnapshot {
            lists: vec![list(REMINDERS, "Reminders")],
            reminders: vec![StoredReminder {
                notes: Some("\r\nline one  \r\n\r\n".into()),
                ..reminder("R1", REMINDERS, "  Cafe\u{301}  ")
            }],
        };
        let session = Session::new(&snapshot);
        let edited = render(&session.document).replace("Cafe\u{301}", "Café");
        let document = parse(&edited).unwrap();
        assert_eq!(
            plan(&snapshot, &session, &document).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn unrepresentable_reminders_are_skipped_and_never_deleted() {
        let mut snapshot = snapshot();
        snapshot
            .reminders
            .push(reminder("D00D0000-1", GROCERIES, "two\nlines"));
        snapshot
            .reminders
            .push(reminder("D00D0000-2", GROCERIES, "rtl \u{202e}override"));
        let session = Session::new(&snapshot);
        assert_eq!(session.skipped, 2);
        let text = render(&session.document);
        assert!(!text.contains("two") && !text.contains("override"));
        assert_eq!(
            plan(&snapshot, &session, &parse(&text).unwrap()).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn each_edit_becomes_exactly_one_change() {
        let edit = only_update(
            plan_after(|t| replace(t, "Review pull request", "Review the pull request")).unwrap(),
        );
        assert_eq!(
            edit,
            ReminderEdit {
                title: Some("Review the pull request".into()),
                ..Default::default()
            }
        );

        let edit =
            only_update(plan_after(|t| replace(t, "- [ ] Oat milk", "- [x] Oat milk")).unwrap());
        assert_eq!(
            edit,
            ReminderEdit {
                complete: true,
                ..Default::default()
            }
        );

        let edit = only_update(plan_after(|t| replace(t, "2026-10-29", "2026-10-30")).unwrap());
        assert_eq!(edit.due, FieldEdit::Set(date(2026, 10, 30)));

        let edit = only_update(plan_after(|t| replace(t, " 🔼 📅", " 📅")).unwrap());
        assert_eq!(edit.priority, FieldEdit::Remove);

        let edit =
            only_update(plan_after(|t| replace(t, "Second paragraph", "Second para")).unwrap());
        assert_eq!(
            edit.notes,
            FieldEdit::Set("Bring the checkbook\n\nSecond para".into())
        );

        let edit = only_update(
            plan_after(|t| {
                let t = replace(t, "- [ ] Oat milk 🆔 9c02d1aa\n", "");
                replace(
                    t,
                    "## Someday 🆔 cccc3333\n",
                    "## Someday 🆔 cccc3333\n\n- [ ] Oat milk 🆔 9c02d1aa\n",
                )
            })
            .unwrap(),
        );
        assert_eq!(edit.list_id.as_deref(), Some(SOMEDAY));
    }

    #[test]
    fn new_and_missing_lines_add_and_delete() {
        let plan = plan_after(|t| {
            replace(
                t,
                "- [ ] Oat milk",
                "- [ ] Eggs 📅 2026-11-01\n- [ ] Oat milk",
            )
        })
        .unwrap();
        assert_eq!(
            plan.changes,
            [ReminderChange::Add {
                list_id: GROCERIES.into(),
                title: "Eggs".into(),
                notes: None,
                due: Some(date(2026, 11, 1)),
                priority: None,
            }]
        );

        let plan = plan_after(|t| replace(t, "- [ ] Oat milk 🆔 9c02d1aa\n", "")).unwrap();
        assert!(
            matches!(&plan.changes[..], [ReminderChange::Delete { before }] if before.title == "Oat milk")
        );
        assert_eq!(plan.warnings, Vec::<String>::new());
    }

    #[test]
    fn a_line_that_lost_its_id_is_flagged() {
        let plan = plan_after(|t| replace(t, "Oat milk 🆔 9c02d1aa", "oat milk")).unwrap();
        assert_eq!(plan.deletions(), 1);
        assert!(
            plan.warnings[0].contains("lost its 🆔"),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn unsafe_files_are_rejected() {
        type Edit = fn(String) -> String;
        let cases: [(&str, Edit); 7] = [
            ("no reminders left", |t| {
                t.lines()
                    .filter(|l| !l.starts_with("- [") && !l.starts_with("    "))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            ("also on line", |t| {
                replace(
                    t,
                    "- [ ] Oat milk 🆔 9c02d1aa",
                    "- [ ] Oat milk 🆔 9c02d1aa\n- [ ] Oat milk 🆔 9c02d1aa",
                )
            }),
            ("no reminder in this file has the 🆔 ffffffff", |t| {
                replace(t, "9c02d1aa", "ffffffff")
            }),
            ("no list has the 🆔 ffffffff", |t| {
                replace(t, "bbbb2222", "ffffffff")
            }),
            ("already has a heading", |t| {
                replace(t, "## Someday 🆔 cccc3333", "## Someday 🆔 bbbb2222")
            }),
            ("can't start out completed", |t| {
                replace(t, "- [ ] Oat milk", "- [x] New\n- [ ] Oat milk")
            }),
            ("needs a title", |t| {
                replace(t, "- [ ] Oat milk", "- [ ]\n- [ ] Oat milk")
            }),
        ];
        for (message, change) in cases {
            let err = plan_after(change).unwrap_err();
            assert!(err.message.contains(message), "{message}: {}", err.message);
        }
    }

    #[test]
    fn every_original_list_heading_must_remain() {
        let snapshot = snapshot();
        let session = Session::new(&snapshot);
        let mut edited = session.document.clone();
        edited.sections.remove(1);
        let error = plan(&snapshot, &session, &edited).unwrap_err();
        assert!(error.message.contains("original list heading"));
        assert!(error.message.contains("Groceries"));
        assert!(error.message.contains("bbbb2222"));
    }

    #[test]
    fn read_only_lists_cannot_change() {
        let mut snapshot = snapshot();
        snapshot.lists[1].writable = false;
        let session = Session::new(&snapshot);
        let edited = render(&session.document).replace("- [ ] Oat milk", "- [x] Oat milk");
        let err = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap_err();
        assert!(err.message.contains("read-only"), "{}", err.message);
    }

    #[test]
    fn renaming_a_heading_warns_and_changes_nothing() {
        let plan = plan_after(|t| replace(t, "## Groceries", "## Food")).unwrap();
        assert_eq!(plan.changes, Vec::<ReminderChange>::new());
        assert!(plan.warnings[0].contains("renaming lists isn't supported"));
    }

    #[test]
    fn describes_the_plan_before_applying() {
        let mut snapshot = snapshot();
        snapshot.reminders[2].alarms_present = true;
        snapshot.reminders[2].recurrence_present = true;
        let session = Session::new(&snapshot);
        let edited = render(&session.document)
            .replace("Review pull request ⏫", "Review the PR ⏫")
            .replace("- [ ] Pay rent", "- [x] Pay rent")
            .replace("- [ ] Oat milk 🆔 9c02d1aa\n", "- [ ] Eggs\n");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        assert_snapshot!(describe(&plan, &snapshot), @r#"
          ~ update    "Review pull request": title → "Review the PR"
          ✓ complete  "Pay rent"
          + add       "Eggs" to Groceries
          - delete    "Oat milk" (loses alarms, repeats)
        "#);
    }

    fn title_strategy() -> impl Strategy<Value = String> {
        proptest::string::string_regex(
            r"[a-zA-Z0-9 #*\[\]\\.\-📅🆔⏫🔼🔽🔺⏬\x{FE0F}\x{3000}é]{0,24}",
        )
        .unwrap()
    }

    fn notes_strategy() -> impl Strategy<Value = String> {
        proptest::string::string_regex(r"([a-z \t\\#*\-📅⏫<!>]{0,12}\n){0,4}[a-z ]{0,6}").unwrap()
    }

    proptest! {
        #[test]
        fn render_then_parse_is_exact(
            items in proptest::collection::vec(
                (
                    title_strategy(),
                    notes_strategy(),
                    any::<bool>(),
                    proptest::option::of(0usize..3),
                    proptest::option::of(0i64..400),
                ),
                0..6,
            ),
            name in title_strategy(),
        ) {
            let priorities = [Priority::Low, Priority::Medium, Priority::High];
            let document = Document {
                sections: vec![Section {
                    line: 0,
                    id: "l1".into(),
                    name: normalize_title(&name),
                    items: items
                        .into_iter()
                        .enumerate()
                        .map(|(i, (title, notes, done, priority, due))| Item {
                            line: 0,
                            id: Some(format!("i{i}")),
                            done,
                            title: normalize_title(&title),
                            priority: priority.map(|p| priorities[p]),
                            due: due.map(|d| date(2026, 1, 1).checked_add(jiff::Span::new().days(d)).unwrap()),
                            notes: normalize_notes(Some(&notes)),
                        })
                        .collect(),
                }],
            };
            let text = render(&document);
            let parsed = parse(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
            prop_assert_eq!(render(&parsed), text);
            prop_assert_eq!(without_lines(parsed), document);
        }
    }
}

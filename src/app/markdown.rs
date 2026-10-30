use std::collections::HashMap;
use std::fmt::Write as _;

use jiff::Zoned;
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::domain::{
    FieldEdit, Priority, ReminderChange, ReminderDueComponents, ReminderEdit, ReminderSnapshot,
    StoredReminder, local_time_zone_name_at, time_zone_is_local_at,
};

const HELP: &str = "<!-- nib task edit
Edit titles, due dates and times (📅 YYYY-MM-DD HH:MM), and priority (⏫ high, 🔼 medium, 🔽 low).
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
    pub due: Option<DueMarker>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueMarker {
    pub date: Date,
    pub time: Option<Time>,
    pub time_zone: Option<String>,
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
    local_zone: TimeZone,
}

impl Session {
    #[cfg(test)]
    pub fn new(snapshot: &ReminderSnapshot) -> Self {
        Self::new_in_zone(snapshot, TimeZone::UTC)
    }

    pub fn new_at(snapshot: &ReminderSnapshot, now: &Zoned) -> Self {
        Self::new_in_zone(snapshot, now.time_zone().clone())
    }

    fn new_in_zone(snapshot: &ReminderSnapshot, local_zone: TimeZone) -> Self {
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
                    due: due_marker(reminder, Some(&local_zone)),
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
            local_zone,
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
    if let Some(due) = &item.due {
        write!(&mut line, " {DUE} {}", due.date).expect("writing to a String cannot fail");
        if let Some(time) = due.time {
            write!(&mut line, " {}", time.strftime("%H:%M"))
                .expect("writing to a String cannot fail");
            if let Some(zone) = &due.time_zone {
                write!(&mut line, " {zone}").expect("writing to a String cannot fail");
            }
        }
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
    due: Option<DueMarker>,
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
    Due(DueMarker),
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
    match strip_variation(marker.trim_start_matches('\\')) {
        ID if marker.starts_with('\\') => return Ok(None),
        ID if valid_id(word) => return Ok(Some((Token::Id(word.to_lowercase()), before_marker))),
        ID => return Err(format!("\"{word}\" isn't a valid 🆔")),
        _ => {}
    }
    if item && let Some((before, marker, fields)) = due_tail_parts(text) {
        if marker.starts_with('\\') {
            return Ok(None);
        }
        return parse_due_marker(&fields).map(|due| Some((Token::Due(due), before)));
    }
    Ok(None)
}

fn tail_marker(text: &str, item: bool) -> Option<(usize, usize)> {
    let (before, word) = last_word(text)?;
    let bare = word.trim_start_matches('\\');
    if item && read_priority(bare).is_some() {
        return Some((offset(text, word), word.len() - bare.len()));
    }
    let (_, marker) = last_word(before)?;
    let bare = marker.trim_start_matches('\\');
    if strip_variation(bare) == ID {
        return Some((offset(text, marker), marker.len() - bare.len()));
    }
    if item && let Some((_, marker, _)) = due_tail_parts(text) {
        let bare = marker.trim_start_matches('\\');
        return Some((offset(text, marker), marker.len() - bare.len()));
    }
    None
}

fn due_tail_parts(text: &str) -> Option<(&str, &str, Vec<&str>)> {
    let mut rest = text;
    let mut fields = Vec::new();
    for _ in 0..4 {
        let (before, word) = last_word(rest)?;
        let bare = word.trim_start_matches('\\');
        if strip_variation(bare) == DUE {
            if fields.is_empty() {
                return None;
            }
            fields.reverse();
            return Some((before, word, fields));
        }
        fields.push(word);
        rest = before;
    }
    None
}

fn parse_due_marker(fields: &[&str]) -> Result<DueMarker, String> {
    let [date, rest @ ..] = fields else {
        return Err("a due marker needs a date".into());
    };
    let date =
        parse_date(date).ok_or_else(|| format!("\"{date}\" isn't a due date; use YYYY-MM-DD"))?;
    let (time, time_zone) = match rest {
        [] => (None, None),
        [time] => (Some(parse_marker_time(time)?), None),
        [time, zone] => (Some(parse_marker_time(time)?), Some((*zone).to_owned())),
        _ => return Err("use 📅 YYYY-MM-DD, optionally followed by HH:MM and an IANA zone".into()),
    };
    Ok(DueMarker {
        date,
        time,
        time_zone,
    })
}

fn parse_marker_time(word: &str) -> Result<Time, String> {
    let shaped = word.len() == 5
        && word.as_bytes().get(2) == Some(&b':')
        && word
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 2 || byte.is_ascii_digit());
    if !shaped {
        return Err(format!("\"{word}\" isn't a due time; use HH:MM"));
    }
    word.parse()
        .map_err(|_| format!("\"{word}\" isn't a valid due time"))
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

fn due_marker(reminder: &StoredReminder, local_zone: Option<&TimeZone>) -> Option<DueMarker> {
    let date = reminder.due?;
    let parts = reminder.due_components.as_ref();
    let time = parts.and_then(due_time);
    let time_zone = time.and_then(|time| {
        parts
            .and_then(|parts| parts.time_zone.clone())
            .filter(|zone| {
                is_single_token(zone)
                    && !local_zone
                        .is_some_and(|local| time_zone_is_local_at(zone, local, date, time))
            })
    });
    Some(DueMarker {
        date,
        time,
        time_zone,
    })
}

fn is_single_token(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_whitespace)
}

fn due_time(parts: &ReminderDueComponents) -> Option<Time> {
    Time::new(
        parts.hour?.try_into().ok()?,
        parts.minute?.try_into().ok()?,
        0,
        0,
    )
    .ok()
}

fn actual_due_marker(reminder: &StoredReminder) -> Option<DueMarker> {
    let mut marker = due_marker(reminder, None)?;
    marker.time_zone = marker.time.and_then(|_| {
        reminder
            .due_components
            .as_ref()
            .and_then(|parts| parts.time_zone.clone())
    });
    Some(marker)
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
    pub(crate) time_zones: Vec<(usize, String)>,
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
                let due_time_zone = new_due_time_zone(item.due.as_ref(), &session.local_zone)
                    .map_err(|message| error(item.line, message))?;
                record_time_zone(&mut plan, item.line, due_time_zone.as_deref());
                plan.changes.push(ReminderChange::Add {
                    list_id: list.id.clone(),
                    title: item.title.clone(),
                    notes: item.notes.clone(),
                    due: item.due.as_ref().map(|due| due.date),
                    due_time: item.due.as_ref().and_then(|due| due.time),
                    due_time_zone,
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
            let (due, due_components) = plan_due(item.due.as_ref(), before, &session.local_zone)
                .map_err(|message| error(item.line, message))?;
            let edit = ReminderEdit {
                title: (!same(&item.title, &normalize_title(&before.title)))
                    .then(|| item.title.clone()),
                notes: if same_notes(item.notes.as_deref(), before.notes.as_deref()) {
                    FieldEdit::Unchanged
                } else {
                    item.notes.clone().map_or(FieldEdit::Remove, FieldEdit::Set)
                },
                due,
                due_components,
                priority: field_edit(item.priority, before.priority),
                list_id: (list.id != before.list_id).then(|| list.id.clone()),
                completed: item.done.then_some(true),
            };
            if edit == ReminderEdit::default() {
                continue;
            }
            record_changed_time_zone(&mut plan, item.line, before, &edit);
            let source = snapshot.lists.iter().find(|l| l.id == before.list_id);
            if source.is_some_and(|l| !l.writable) || (edit.list_id.is_some() && !list.writable) {
                return Err(error(item.line, "that change touches a read-only list"));
            }
            if edit.due.is_changed() || edit.due_components.is_some() {
                for alarm in before.absolute_alarms.as_deref().unwrap_or_default() {
                    let local = alarm.to_zoned(session.local_zone.clone());
                    plan.warnings.push(format!(
                        "line {}: \"{}\" has an alarm that stays at {}",
                        item.line,
                        item.title,
                        local.strftime("%Y-%m-%d %H:%M")
                    ));
                }
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

fn record_changed_time_zone(
    plan: &mut Plan,
    line: usize,
    before: &StoredReminder,
    edit: &ReminderEdit,
) {
    let before_zone = before
        .due_components
        .as_ref()
        .and_then(|parts| parts.time_zone.as_deref());
    let zone = edit
        .due_components
        .as_deref()
        .and_then(|parts| parts.time_zone.as_deref())
        .filter(|zone| before_zone != Some(*zone));
    record_time_zone(plan, line, zone);
}

fn record_time_zone(plan: &mut Plan, line: usize, zone: Option<&str>) {
    if let Some(zone) = zone {
        plan.time_zones.push((line, zone.to_owned()));
    }
}

fn plan_due(
    after: Option<&DueMarker>,
    before: &StoredReminder,
    local_zone: &TimeZone,
) -> Result<(FieldEdit<Date>, Option<Box<ReminderDueComponents>>), String> {
    let Some(after) = after else {
        return Ok((
            before
                .due
                .map_or(FieldEdit::Unchanged, |_| FieldEdit::Remove),
            None,
        ));
    };
    let before_marker = actual_due_marker(before);
    let displayed_before = due_marker(before, Some(local_zone));
    if displayed_before.as_ref() == Some(after) {
        return Ok((FieldEdit::Unchanged, None));
    }
    let before_time = before_marker.as_ref().and_then(|due| due.time);
    let before_zone = before_marker
        .as_ref()
        .and_then(|due| due.time_zone.as_deref());
    let before_zone_visible = displayed_before
        .as_ref()
        .and_then(|due| due.time_zone.as_deref())
        .is_some();
    let before_zone_was_local = before_marker.as_ref().is_some_and(|due| {
        due.time
            .zip(due.time_zone.as_deref())
            .is_some_and(|(time, zone)| time_zone_is_local_at(zone, local_zone, due.date, time))
    });
    let before_zone_stays_hidden = !before_zone_visible && !before_zone_was_local;
    let zone = match (after.time, after.time_zone.as_deref()) {
        (None, _) => None,
        (Some(_), Some(zone)) if Some(zone) == before_zone => Some(zone.to_owned()),
        (Some(_), Some(zone)) => Some(canonical_zone(zone)?),
        (Some(_), None) if before_time.is_some() && before_zone_stays_hidden => {
            before_zone.map(ToOwned::to_owned)
        }
        (Some(time), None) => Some(local_time_zone_name_at(local_zone, after.date, time)),
    };
    let exact_after = DueMarker {
        date: after.date,
        time: after.time,
        time_zone: zone.clone(),
    };
    if before_marker.as_ref() == Some(&exact_after) {
        return Ok((FieldEdit::Unchanged, None));
    }
    let due = field_edit(Some(after.date), before.due);
    if before.due_components.is_none() && after.time.is_none() {
        return Ok((due, None));
    }
    let mut parts = before.due_components.clone().unwrap_or_default();
    parts.year = Some(after.date.year().into());
    parts.month = Some(after.date.month().into());
    parts.day = Some(after.date.day().into());
    if let Some(time) = after.time {
        parts.hour = Some(time.hour().into());
        parts.minute = Some(time.minute().into());
        if Some(time) != before_time {
            parts.second = Some(0);
            parts.nanosecond = Some(0);
        }
        parts.time_zone = zone;
    } else if before_time.is_some() {
        parts.hour = None;
        parts.minute = None;
        parts.second = None;
        parts.nanosecond = None;
        parts.time_zone = None;
    }
    Ok((due, Some(Box::new(parts))))
}

fn canonical_zone(zone: &str) -> Result<String, String> {
    TimeZone::get(zone)
        .map_err(|_| format!("\"{zone}\" isn't an IANA time zone"))?
        .iana_name()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("\"{zone}\" isn't an IANA time zone"))
}

fn new_due_time_zone(
    due: Option<&DueMarker>,
    local_zone: &TimeZone,
) -> Result<Option<String>, String> {
    match due {
        Some(DueMarker {
            time: Some(_),
            time_zone: Some(zone),
            ..
        }) => canonical_zone(zone).map(Some),
        Some(DueMarker {
            date,
            time: Some(time),
            ..
        }) => Ok(Some(local_time_zone_name_at(local_zone, *date, *time))),
        _ => Ok(None),
    }
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
            ReminderChange::Restore { before } => {
                format!("  + restore   \"{}\"", normalize_title(&before.title))
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
                if edit.due.is_changed() || edit.due_components.is_some() {
                    let old = actual_due_marker(before).map(|due| format_due_marker(&due));
                    let new = due_after_edit(before, edit).map(|due| format_due_marker(&due));
                    match (old, new) {
                        (Some(old), Some(new)) => parts.push(format!("due {old} → {new}")),
                        (None, Some(new)) => parts.push(format!("due {new}")),
                        (Some(_), None) => parts.push("due date removed".into()),
                        (None, None) => {}
                    }
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
                if edit.completed == Some(true) && parts.is_empty() {
                    format!("  ✓ complete  \"{title}\"")
                } else {
                    if edit.completed == Some(true) {
                        parts.push("completed".into());
                    } else if edit.completed == Some(false) {
                        parts.push("marked incomplete".into());
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

fn due_after_edit(before: &StoredReminder, edit: &ReminderEdit) -> Option<DueMarker> {
    let mut after = before.clone();
    match edit.due {
        FieldEdit::Unchanged => {}
        FieldEdit::Set(date) => after.due = Some(date),
        FieldEdit::Remove => {
            after.due = None;
            after.due_components = None;
        }
    }
    if let Some(parts) = &edit.due_components {
        after.due_components = Some((**parts).clone());
    }
    actual_due_marker(&after)
}

fn format_due_marker(due: &DueMarker) -> String {
    let mut value = due.date.to_string();
    if let Some(time) = due.time {
        write!(&mut value, " {}", time.strftime("%H:%M")).expect("writing to a String cannot fail");
        if let Some(zone) = &due.time_zone {
            write!(&mut value, " {zone}").expect("writing to a String cannot fail");
        }
    }
    value
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
    use crate::test_support::named_fixed_zone;

    const REMINDERS: &str = "AAAA1111-0000-0000-0000-000000000001";
    const GROCERIES: &str = "BBBB2222-0000-0000-0000-000000000002";
    const SOMEDAY: &str = "CCCC3333-0000-0000-0000-000000000003";

    fn chicago_zone() -> TimeZone {
        TimeZone::posix("CST6CDT,M3.2.0,M11.1.0").unwrap()
    }

    fn named_chicago_zone() -> TimeZone {
        named_fixed_zone("America/Chicago", -6 * 60 * 60, *b"CST\0")
    }

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
            completed: false,
            modified: None,
            alarms_present: false,
            absolute_alarms: Some(Vec::new()),
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
        Edit titles, due dates and times (📅 YYYY-MM-DD HH:MM), and priority (⏫ high, 🔼 medium, 🔽 low).
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
            (
                "Keep 📅 2026-01-01 09:00 America/New_York",
                "Keep \\📅 2026-01-01 09:00 America/New_York",
            ),
            ("Keep \\📅 2026-01-01", "Keep \\\\📅 2026-01-01"),
            ("Report 🔼 📅 2026-01-01", "Report 🔼 \\📅 2026-01-01"),
            ("Plan 📅 someday", "Plan \\📅 someday"),
            ("Urgent ⏫", "Urgent \\⏫"),
            ("Urgent ⏫\u{fe0f}", "Urgent \\⏫\u{fe0f}"),
            ("Due 📅 \\⏫", "Due 📅 \\\\⏫"),
            ("Use the 🆔 badge", "Use the \\🆔 badge"),
            ("x 📅 🆔 a", "x 📅 \\🆔 a"),
            ("Q 📅 b 🆔 c", "Q 📅 b \\🆔 c"),
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
        assert_eq!(
            item.due,
            Some(DueMarker {
                date: date(2026, 10, 29),
                time: None,
                time_zone: None,
            })
        );
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
                completed: Some(true),
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
                due_time: None,
                due_time_zone: None,
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

    fn timed_reminder(
        id: &str,
        title: &str,
        time: Option<(i64, i64)>,
        zone: Option<&str>,
    ) -> StoredReminder {
        let mut reminder = reminder(id, REMINDERS, title);
        reminder.due = Some(date(2026, 11, 3));
        reminder.due_components = Some(ReminderDueComponents {
            calendar: Some("gregorian".into()),
            time_zone: zone.map(ToOwned::to_owned),
            year: Some(2026),
            month: Some(11),
            day: Some(3),
            hour: time.map(|time| time.0),
            minute: time.map(|time| time.1),
            second: time.map(|_| 15),
            nanosecond: time.map(|_| 123),
            ..ReminderDueComponents::default()
        });
        reminder
    }

    fn timed_snapshot(reminder: StoredReminder) -> ReminderSnapshot {
        ReminderSnapshot {
            lists: vec![list(REMINDERS, "Reminders")],
            reminders: vec![reminder],
        }
    }

    #[test]
    fn due_markers_render_local_floating_and_foreign_zones_without_writes() {
        let snapshot = ReminderSnapshot {
            lists: vec![list(REMINDERS, "Reminders")],
            reminders: vec![
                timed_reminder("local", "Local", Some((9, 0)), Some("America/Chicago")),
                timed_reminder("floating", "Floating", Some((10, 0)), None),
                timed_reminder(
                    "foreign",
                    "Foreign",
                    Some((11, 0)),
                    Some("America/New_York"),
                ),
            ],
        };
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let text = render(&session.document);
        assert!(text.contains("Local 📅 2026-11-03 09:00 🆔"));
        assert!(text.contains("Floating 📅 2026-11-03 10:00 🆔"));
        assert!(text.contains("Foreign 📅 2026-11-03 11:00 America/New_York 🆔"));
        assert_eq!(
            plan(&snapshot, &session, &parse(&text).unwrap()).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn fixed_and_multi_token_stored_zones_round_trip_unchanged() {
        let snapshot = ReminderSnapshot {
            lists: vec![list(REMINDERS, "Reminders")],
            reminders: vec![
                timed_reminder("fixed", "Fixed", Some((9, 0)), Some("GMT+0100")),
                timed_reminder("spaced", "Spaced", Some((10, 0)), Some("Custom Zone")),
            ],
        };
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let text = render(&session.document);
        assert!(text.contains("Fixed 📅 2026-11-03 09:00 GMT+0100 🆔"));
        assert!(text.contains("Spaced 📅 2026-11-03 10:00 🆔"));
        assert_eq!(
            plan(&snapshot, &session, &parse(&text).unwrap()).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn changing_a_time_preserves_a_hidden_multi_token_zone() {
        let snapshot = timed_snapshot(timed_reminder(
            "spaced",
            "Spaced",
            Some((10, 0)),
            Some("Eastern Standard Time"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("10:00", "11:00");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!((parts.hour, parts.minute), (Some(11), Some(0)));
        assert_eq!(parts.time_zone.as_deref(), Some("Eastern Standard Time"));
    }

    #[test]
    fn component_edits_preserve_every_untouched_due_component() {
        let original = timed_reminder(
            "timed-1",
            "Call team",
            Some((9, 30)),
            Some("America/New_York"),
        );
        let cases = [
            (
                "2026-11-03 09:30 America/New_York",
                "2026-11-04 09:30 America/New_York",
            ),
            (
                "2026-11-03 09:30 America/New_York",
                "2026-11-03 10:45 America/New_York",
            ),
        ];
        for (from, to) in cases {
            let snapshot = timed_snapshot(original.clone());
            let session = Session::new_in_zone(&snapshot, named_chicago_zone());
            let edited = render(&session.document).replace(from, to);
            let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
            let parts = edit.due_components.unwrap();
            assert_eq!(parts.calendar.as_deref(), Some("gregorian"));
            assert_eq!(parts.time_zone.as_deref(), Some("America/New_York"));
            if to.contains("2026-11-04") {
                assert_eq!(parts.second, Some(15));
                assert_eq!(parts.nanosecond, Some(123));
                assert_eq!(parts.day, Some(4));
                assert_eq!((parts.hour, parts.minute), (Some(9), Some(30)));
            } else {
                assert_eq!(parts.second, Some(0));
                assert_eq!(parts.nanosecond, Some(0));
                assert_eq!(parts.day, Some(3));
                assert_eq!((parts.hour, parts.minute), (Some(10), Some(45)));
            }
        }
    }

    #[test]
    fn removing_and_adding_times_have_explicit_zone_semantics() {
        let timed = timed_reminder(
            "timed-1",
            "Call team",
            Some((9, 30)),
            Some("America/New_York"),
        );
        let snapshot = timed_snapshot(timed);
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited =
            render(&session.document).replace("2026-11-03 09:30 America/New_York", "2026-11-03");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!(
            (parts.hour, parts.minute, parts.time_zone),
            (None, None, None)
        );
        assert_eq!(parts.day, Some(3));

        let date_only = timed_reminder("date-1", "File report", None, None);
        let snapshot = timed_snapshot(date_only);
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("2026-11-03", "2026-11-03 09:00");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!((parts.hour, parts.minute), (Some(9), Some(0)));
        assert_eq!(parts.time_zone.as_deref(), Some("America/Chicago"));
    }

    #[test]
    fn removing_a_foreign_zone_suffix_pins_to_the_local_zone() {
        let snapshot = timed_snapshot(timed_reminder(
            "timed-1",
            "Call team",
            Some((9, 30)),
            Some("America/New_York"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace(" America/New_York", "");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        assert_eq!(
            edit.due_components.unwrap().time_zone.as_deref(),
            Some("America/Chicago")
        );
        assert!(description.contains("2026-11-03 09:30 America/Chicago"));
    }

    #[test]
    fn edited_zone_names_are_canonicalized() {
        let snapshot = timed_snapshot(timed_reminder(
            "timed-1",
            "Call team",
            Some((9, 30)),
            Some("America/Chicago"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document)
            .replace("2026-11-03 09:30", "2026-11-03 09:30 america/new_york");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        assert_eq!(
            edit.due_components.unwrap().time_zone.as_deref(),
            Some("America/New_York")
        );
    }

    #[test]
    fn unnamed_local_zones_pin_new_times_to_the_due_time_offset() {
        let mut reminder = timed_reminder("date-1", "File report", None, None);
        reminder.due = Some(date(2026, 12, 1));
        reminder.due_components.as_mut().unwrap().month = Some(12);
        reminder.due_components.as_mut().unwrap().day = Some(1);
        let snapshot = timed_snapshot(reminder);
        let now = date(2026, 10, 26)
            .at(8, 0, 0, 0)
            .to_zoned(chicago_zone())
            .unwrap();
        let session = Session::new_at(&snapshot, &now);
        let edited = render(&session.document).replace("2026-12-01", "2026-12-01 09:00");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        assert_eq!(
            edit.due_components.unwrap().time_zone.as_deref(),
            Some("GMT-0600")
        );
        assert!(description.contains("2026-12-01 09:00 GMT-0600"));
    }

    #[test]
    fn offset_zones_are_hidden_only_when_local_at_the_due_time() {
        let mut local = timed_reminder("local", "Local", Some((9, 0)), Some("GMT-0600"));
        local.due = Some(date(2026, 12, 1));
        local.due_components.as_mut().unwrap().month = Some(12);
        local.due_components.as_mut().unwrap().day = Some(1);
        let mut foreign = local.clone();
        foreign.id = "foreign".into();
        foreign.title = "Foreign".into();
        foreign.due_components.as_mut().unwrap().time_zone = Some("GMT-0500".into());
        let snapshot = ReminderSnapshot {
            lists: vec![list(REMINDERS, "Reminders")],
            reminders: vec![local, foreign],
        };
        let now = date(2026, 10, 26)
            .at(8, 0, 0, 0)
            .to_zoned(chicago_zone())
            .unwrap();
        let text = render(&Session::new_at(&snapshot, &now).document);
        assert!(text.contains("Local 📅 2026-12-01 09:00 🆔"));
        assert!(text.contains("Foreign 📅 2026-12-01 09:00 GMT-0500 🆔"));
    }

    #[test]
    fn floating_time_edits_stay_floating() {
        let snapshot = timed_snapshot(timed_reminder("float-1", "Call", Some((9, 0)), None));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("09:00", "10:00");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        assert_eq!(edit.due_components.unwrap().time_zone, None);
    }

    #[test]
    fn hidden_partial_time_components_survive_a_date_edit() {
        let mut partial = timed_reminder("partial-1", "Odd reminder", None, None);
        let parts = partial.due_components.as_mut().unwrap();
        parts.hour = Some(9);
        parts.time_zone = Some("America/New_York".into());
        let snapshot = timed_snapshot(partial);
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("2026-11-03", "2026-11-04");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!(parts.day, Some(4));
        assert_eq!(parts.hour, Some(9));
        assert_eq!(parts.minute, None);
        assert_eq!(parts.time_zone.as_deref(), Some("America/New_York"));
    }

    fn timed_reminder_on(
        id: &str,
        on: Date,
        time: (i64, i64),
        zone: Option<&str>,
    ) -> StoredReminder {
        let mut reminder = timed_reminder(id, "Call", Some(time), zone);
        reminder.due = Some(on);
        let parts = reminder.due_components.as_mut().unwrap();
        parts.year = Some(on.year().into());
        parts.month = Some(on.month().into());
        parts.day = Some(on.day().into());
        reminder
    }

    fn october_session(snapshot: &ReminderSnapshot) -> Session {
        let now = date(2026, 10, 26)
            .at(8, 0, 0, 0)
            .to_zoned(chicago_zone())
            .unwrap();
        Session::new_at(snapshot, &now)
    }

    #[test]
    fn hidden_unnamed_local_zone_repins_to_the_new_offset_when_a_date_edit_crosses_dst() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = october_session(&snapshot);
        let edited = render(&session.document).replace("2026-12-01 09:00", "2027-06-01 09:00");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        let parts = edit.due_components.unwrap();
        assert_eq!(
            (parts.year, parts.month, parts.day),
            (Some(2027), Some(6), Some(1))
        );
        assert_eq!((parts.hour, parts.minute), (Some(9), Some(0)));
        assert_eq!(parts.time_zone.as_deref(), Some("GMT-0500"));
        assert_snapshot!(description, @r#"  ~ update    "Call": due 2026-12-01 09:00 GMT-0600 → 2027-06-01 09:00 GMT-0500"#);
    }

    #[test]
    fn hidden_named_local_zone_repins_to_its_name_when_the_date_changes() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("2026-12-01 09:00", "2027-06-01 09:00");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        let parts = edit.due_components.unwrap();
        assert_eq!((parts.hour, parts.minute), (Some(9), Some(0)));
        assert_eq!(parts.time_zone.as_deref(), Some("America/Chicago"));
        assert_snapshot!(description, @r#"  ~ update    "Call": due 2026-12-01 09:00 GMT-0600 → 2027-06-01 09:00 America/Chicago"#);
    }

    #[test]
    fn hidden_named_local_zone_repins_to_its_name_when_no_dst_change_applies() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("2026-12-01", "2026-12-15");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        let parts = edit.due_components.unwrap();
        assert_eq!(parts.day, Some(15));
        assert_eq!(parts.time_zone.as_deref(), Some("America/Chicago"));
        assert_snapshot!(description, @r#"  ~ update    "Call": due 2026-12-01 09:00 GMT-0600 → 2026-12-15 09:00 America/Chicago"#);
    }

    #[test]
    fn visible_zone_stays_when_a_date_edit_crosses_dst() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "visible",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0500"),
        ));
        let session = october_session(&snapshot);
        let edited = render(&session.document)
            .replace("2026-12-01 09:00 GMT-0500", "2027-06-01 09:00 GMT-0500");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!(parts.day, Some(1));
        assert_eq!(parts.month, Some(6));
        assert_eq!(parts.time_zone.as_deref(), Some("GMT-0500"));
    }

    #[test]
    fn hidden_unnamed_local_zone_keeps_its_offset_name_when_no_dst_change_applies() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = october_session(&snapshot);
        let edited = render(&session.document).replace("2026-12-01", "2026-12-15");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!(parts.day, Some(15));
        assert_eq!(parts.time_zone.as_deref(), Some("GMT-0600"));
    }

    #[test]
    fn hidden_unnamed_local_zone_repins_when_a_time_edit_crosses_dst_on_the_same_day() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 11, 1),
            (0, 30),
            Some("GMT-0500"),
        ));
        let session = october_session(&snapshot);
        let edited = render(&session.document).replace("2026-11-01 00:30", "2026-11-01 03:30");
        let edit = only_update(plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap());
        let parts = edit.due_components.unwrap();
        assert_eq!((parts.hour, parts.minute), (Some(3), Some(30)));
        assert_eq!(parts.time_zone.as_deref(), Some("GMT-0600"));
    }

    #[test]
    fn hidden_named_local_zone_repins_to_its_name_when_only_the_time_changes() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("2026-12-01 09:00", "2026-12-01 10:30");
        let plan = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        let description = describe(&plan, &snapshot);
        let edit = only_update(plan);
        let parts = edit.due_components.unwrap();
        assert_eq!((parts.hour, parts.minute), (Some(10), Some(30)));
        assert_eq!(parts.time_zone.as_deref(), Some("America/Chicago"));
        assert_snapshot!(description, @r#"  ~ update    "Call": due 2026-12-01 09:00 GMT-0600 → 2026-12-01 10:30 America/Chicago"#);
    }

    #[test]
    fn unchanged_hidden_local_zone_is_left_alone() {
        let snapshot = timed_snapshot(timed_reminder_on(
            "hidden",
            date(2026, 12, 1),
            (9, 0),
            Some("GMT-0600"),
        ));
        let session = october_session(&snapshot);
        let edited = render(&session.document);
        assert_eq!(
            plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap(),
            Plan::default()
        );
    }

    #[test]
    fn absolute_alarm_warnings_name_the_time_but_relative_alarms_do_not_warn() {
        let mut absolute = timed_reminder("alarm-1", "Wake up", Some((9, 0)), None);
        absolute.alarms_present = true;
        absolute.absolute_alarms = Some(vec!["2026-11-03T14:00:00Z".parse().unwrap()]);
        let snapshot = timed_snapshot(absolute);
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("09:00", "10:00");
        let planned = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap();
        assert_eq!(planned.warnings.len(), 1);
        assert!(
            planned.warnings[0].contains("\"Wake up\" has an alarm that stays at 2026-11-03 08:00")
        );

        let mut relative = snapshot.reminders[0].clone();
        relative.absolute_alarms = Some(Vec::new());
        let snapshot = timed_snapshot(relative);
        let session = Session::new_in_zone(&snapshot, named_chicago_zone());
        let edited = render(&session.document).replace("09:00", "10:00");
        assert_eq!(
            plan(&snapshot, &session, &parse(&edited).unwrap())
                .unwrap()
                .warnings,
            Vec::<String>::new()
        );
    }

    #[test]
    fn due_marker_times_and_zones_are_strict() {
        let cases = [("9:00", "use HH:MM"), ("24:00", "valid due time")];
        for (tail, message) in cases {
            let error =
                parse(&format!("## L 🆔 l1\n- [ ] Call 📅 2026-11-03 {tail}\n")).unwrap_err();
            assert!(error.message.contains(message), "{tail}: {error}");
        }

        let snapshot = timed_snapshot(timed_reminder("zone", "Call", None, None));
        let session = Session::new(&snapshot);
        let edited = render(&session.document).replace("2026-11-03", "2026-11-03 09:00 Not/A-Zone");
        let error = plan(&snapshot, &session, &parse(&edited).unwrap()).unwrap_err();
        assert!(error.message.contains("isn't an IANA time zone"));
    }

    fn title_strategy() -> impl Strategy<Value = String> {
        proptest::string::string_regex(
            r"[a-zA-Z0-9 #*\[\]\\.\-:/📅🆔⏫🔼🔽🔺⏬\x{FE0F}\x{3000}é]{0,24}",
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
                            due: due.map(|d| DueMarker {
                                date: date(2026, 1, 1).checked_add(jiff::Span::new().days(d)).unwrap(),
                                time: None,
                                time_zone: None,
                            }),
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

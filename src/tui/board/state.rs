use std::cell::Cell;

use jiff::Zoned;

use crate::app::filter::{Filter, FilterKind, FilterRow};
use crate::domain::keymap::{Action, State as InputState};
use crate::domain::{Config, Event, NewReminder, Priority, TaskChange};
use crate::tui::state::{
    ConfirmKey, ConfirmStep, Data, Effect as SessionEffect, Item, ListChoice, NOTHING_DELETED,
    PRIORITIES, Response, Status, confirm_delete, delete_needs_count, is_plain_text,
};

pub(crate) const PAGE: usize = 10;

pub(crate) const ACTS: [(char, &str); 6] = [
    ('x', "done"),
    ('r', "reopen"),
    ('p', "priority"),
    ('m', "move"),
    ('e', "edit"),
    ('d', "delete"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Lists,
    Items,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overlay {
    Help,
    Act,
    Priority,
    Move,
    ConfirmDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEdit {
    Insert(char),
    Backspace,
    DeleteForward,
    KillWord,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKey {
    Char(char),
    Up,
    Down,
    Accept,
    Cancel,
    Backspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Input {
    Action(Action),
    Text(TextEdit),
    Menu(MenuKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    Session(SessionEffect),
    Edit(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    All,
    Today,
    List(String),
    Events,
    View(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct View {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Row<'a> {
    Reminder(&'a Item),
    Event(&'a Event),
    Add(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapturePriority {
    Parsed,
    Set(Option<Priority>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowIndex {
    Reminder(usize),
    Event(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub text: String,
    pub caret: usize,
}

impl Line {
    fn edit(&mut self, edit: TextEdit) -> bool {
        let length = self.text.chars().count();
        match edit {
            TextEdit::Insert(character) => {
                let at = self.offset(self.caret);
                self.text.insert(at, character);
                self.caret += 1;
            }
            TextEdit::Backspace if self.caret > 0 => {
                self.caret -= 1;
                let at = self.offset(self.caret);
                self.text.remove(at);
            }
            TextEdit::DeleteForward if self.caret < length => {
                let at = self.offset(self.caret);
                self.text.remove(at);
            }
            TextEdit::KillWord if self.caret > 0 => {
                let characters: Vec<char> = self.text.chars().collect();
                let mut start = self.caret;
                while start > 0 && characters[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !characters[start - 1].is_whitespace() {
                    start -= 1;
                }
                let from = self.offset(start);
                let to = self.offset(self.caret);
                self.text.replace_range(from..to, "");
                self.caret = start;
            }
            TextEdit::Left => {
                self.caret = self.caret.saturating_sub(1);
                return false;
            }
            TextEdit::Right => {
                self.caret = (self.caret + 1).min(length);
                return false;
            }
            _ => return false,
        }
        true
    }

    fn set(&mut self, text: String) {
        self.caret = text.chars().count();
        self.text = text;
    }

    fn offset(&self, characters: usize) -> usize {
        self.text
            .char_indices()
            .nth(characters)
            .map_or(self.text.len(), |(offset, _)| offset)
    }
}

pub(crate) struct Board {
    now: Zoned,
    config: Config,
    data: Data,
    events: Vec<Event>,
    views: Vec<View>,
    mode: InputState,
    overlay: Option<Overlay>,
    focus: Pane,
    preview: bool,
    scope: Scope,
    applied: String,
    query: Line,
    filter_error: Option<String>,
    capture: Line,
    capture_priority: CapturePriority,
    preview_add: Option<Result<NewReminder, String>>,
    filter: Filter,
    rows: Vec<RowIndex>,
    cursor: usize,
    selected: Vec<String>,
    targets: Vec<String>,
    menu_cursor: usize,
    help_scroll: usize,
    help_limit: Cell<usize>,
    items_offset: Cell<usize>,
    lists_offset: Cell<usize>,
    confirm_input: String,
    status: Option<Status>,
    quit: bool,
    loading_completed: bool,
}

impl Board {
    pub(crate) fn new(
        data: Data,
        mut events: Vec<Event>,
        config: Config,
        now: Zoned,
        status: Option<Status>,
    ) -> (Self, Option<Effect>) {
        events.sort_by(|left, right| left.start.cmp(&right.start));
        let views = config
            .tui
            .views
            .iter()
            .filter(|(_, expression)| Filter::parse(expression).is_ok())
            .map(|(name, expression)| View {
                name: name.clone(),
                expression: expression.clone(),
            })
            .collect();
        let mut board = Self {
            now,
            config,
            data,
            events,
            views,
            mode: InputState::Browse,
            overlay: None,
            focus: Pane::Items,
            preview: true,
            scope: Scope::All,
            applied: String::new(),
            query: Line::default(),
            filter_error: None,
            capture: Line::default(),
            capture_priority: CapturePriority::Parsed,
            preview_add: None,
            filter: Filter::parse("").expect("an empty expression has no tokens to reject"),
            rows: Vec::new(),
            cursor: 0,
            selected: Vec::new(),
            targets: Vec::new(),
            menu_cursor: 0,
            help_scroll: 0,
            help_limit: Cell::new(usize::MAX),
            items_offset: Cell::new(0),
            lists_offset: Cell::new(0),
            confirm_input: String::new(),
            status,
            quit: false,
            loading_completed: false,
        };
        let effect = board.refilter();
        (board, effect)
    }

    pub(crate) fn now(&self) -> &Zoned {
        &self.now
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) fn mode(&self) -> InputState {
        self.mode
    }

    pub(crate) fn overlay(&self) -> Option<Overlay> {
        self.overlay
    }

    pub(crate) fn focus(&self) -> Pane {
        self.focus
    }

    pub(crate) fn preview(&self) -> bool {
        self.preview
    }

    pub(crate) fn quit(&self) -> bool {
        self.quit
    }

    pub(crate) fn includes_completed(&self) -> bool {
        self.data.includes_completed
    }

    pub(crate) fn applied(&self) -> &str {
        &self.applied
    }

    pub(crate) fn query(&self) -> &Line {
        &self.query
    }

    pub(crate) fn capture(&self) -> &Line {
        &self.capture
    }

    pub(crate) fn capture_preview(&self) -> Option<&Result<NewReminder, String>> {
        self.preview_add.as_ref()
    }

    pub(crate) fn filter_error(&self) -> Option<&str> {
        self.filter_error.as_deref()
    }

    pub(crate) fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn scope_cursor(&self) -> usize {
        self.scopes()
            .iter()
            .position(|scope| *scope == self.scope)
            .unwrap_or_default()
    }

    pub(crate) fn menu_cursor(&self) -> usize {
        self.menu_cursor
    }

    pub(crate) fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    pub(crate) fn set_help_limit(&self, limit: usize) {
        self.help_limit.set(limit);
    }

    pub(crate) fn items_offset(&self) -> usize {
        self.items_offset.get()
    }

    pub(crate) fn set_items_offset(&self, offset: usize) {
        self.items_offset.set(offset);
    }

    pub(crate) fn lists_offset(&self) -> usize {
        self.lists_offset.get()
    }

    pub(crate) fn set_lists_offset(&self, offset: usize) {
        self.lists_offset.set(offset);
    }

    pub(crate) fn selected(&self) -> &[String] {
        &self.selected
    }

    pub(crate) fn is_selected(&self, id: &str) -> bool {
        self.selected.iter().any(|selected| selected == id)
    }

    pub(crate) fn targets(&self) -> &[String] {
        &self.targets
    }

    pub(crate) fn confirm_input(&self) -> &str {
        &self.confirm_input
    }

    pub(crate) fn lists(&self) -> &[ListChoice] {
        &self.data.lists
    }

    pub(crate) fn item(&self, id: &str) -> Option<&Item> {
        self.data.items.iter().find(|item| item.id == id)
    }

    pub(crate) fn scopes(&self) -> Vec<Scope> {
        let mut scopes = vec![Scope::All, Scope::Today];
        scopes.extend(self.list_names().into_iter().map(Scope::List));
        if !self.events.is_empty() {
            scopes.push(Scope::Events);
        }
        scopes.extend((0..self.views.len()).map(Scope::View));
        scopes
    }

    pub(crate) fn scope_name(&self, scope: &Scope) -> String {
        match scope {
            Scope::All => "All".into(),
            Scope::Today => "Today".into(),
            Scope::List(name) => name.clone(),
            Scope::Events => "Events".into(),
            Scope::View(index) => self.views[*index].name.clone(),
        }
    }

    pub(crate) fn current_scope(&self) -> Scope {
        self.scope.clone()
    }

    pub(crate) fn scope_count(&self, scope: &Scope) -> usize {
        let Ok(filter) = Filter::parse(self.scope_expression(scope)) else {
            return 0;
        };
        self.matching(&filter, scope).len()
    }

    fn list_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .data
            .lists
            .iter()
            .map(|list| list.name.clone())
            .collect();
        for item in &self.data.items {
            if let Some(list) = &item.list
                && !names.contains(list)
            {
                names.push(list.clone());
            }
        }
        names
    }

    fn scope_expression(&self, scope: &Scope) -> &str {
        match scope {
            Scope::All | Scope::List(_) => "",
            Scope::Today => "due:today",
            Scope::Events => "is:event",
            Scope::View(index) => &self.views[*index].expression,
        }
    }

    pub(crate) fn rows(&self) -> Vec<Row<'_>> {
        let mut rows: Vec<Row<'_>> = self.rows.iter().map(|row| self.row(*row)).collect();
        if let Some(text) = self.add_text() {
            rows.push(Row::Add(text));
        }
        rows
    }

    pub(crate) fn match_count(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn current(&self) -> Option<Row<'_>> {
        if let Some(row) = self.rows.get(self.cursor) {
            Some(self.row(*row))
        } else if self.cursor == self.rows.len() {
            self.add_text().map(Row::Add)
        } else {
            None
        }
    }

    fn row(&self, row: RowIndex) -> Row<'_> {
        match row {
            RowIndex::Reminder(index) => Row::Reminder(&self.data.items[index]),
            RowIndex::Event(index) => Row::Event(&self.events[index]),
        }
    }

    fn add_text(&self) -> Option<&str> {
        if self.mode != InputState::Filter || self.filter_error.is_some() {
            return None;
        }
        let text = self.query.text.trim();
        (!text.is_empty() && is_plain_text(text, self.config.task.parse_inline)).then_some(text)
    }

    fn row_count(&self) -> usize {
        self.rows.len() + usize::from(self.add_text().is_some())
    }

    fn current_reminder(&self) -> Option<&Item> {
        match self.current() {
            Some(Row::Reminder(item)) => Some(item),
            _ => None,
        }
    }

    pub(crate) fn hidden_target_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|id| {
                !self.rows.iter().any(|row| match row {
                    RowIndex::Reminder(index) => self.data.items[*index].id == **id,
                    RowIndex::Event(_) => false,
                })
            })
            .count()
    }

    pub(crate) fn partial_target_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|id| self.item(id).is_some_and(|item| item.backup_partial))
            .count()
    }

    pub(crate) fn delete_needs_count(&self) -> bool {
        delete_needs_count(
            &self.targets,
            self.partial_target_count(),
            self.targets.iter().any(|id| self.item(id).is_none()),
        )
    }

    fn text_expression(&self) -> &str {
        if self.mode == InputState::Filter {
            &self.query.text
        } else {
            &self.applied
        }
    }

    fn matching(&self, filter: &Filter, scope: &Scope) -> Vec<RowIndex> {
        let now = &self.now;
        let list = match scope {
            Scope::List(name) => Some(name.as_str()),
            _ => None,
        };
        let reminders = self
            .data
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| list.is_none_or(|list| item.list.as_deref() == Some(list)))
            .filter(|(_, item)| filter.matches(&reminder_row(item), now))
            .map(|(index, _)| RowIndex::Reminder(index));
        let events = self
            .events
            .iter()
            .enumerate()
            .filter(|_| list.is_none())
            .filter(|(_, event)| filter.matches(&event_row(event, now), now))
            .map(|(index, _)| RowIndex::Event(index));
        reminders.chain(events).collect()
    }

    fn refilter(&mut self) -> Option<Effect> {
        let text = self.text_expression().to_owned();
        match Filter::parse(&text) {
            Ok(_) => {
                self.filter_error = None;
                let scope = self.current_scope();
                let combined = format!("{} {text}", self.scope_expression(&scope));
                self.filter = Filter::parse(&combined)
                    .expect("joining two valid filter expressions keeps every token valid");
            }
            Err(error) => self.filter_error = Some(error.to_string()),
        }
        let scope = self.current_scope();
        self.rows = self.matching(&self.filter, &scope);
        self.cursor = self.cursor.min(self.row_count().saturating_sub(1));
        if self.filter.includes_completed()
            && !self.data.includes_completed
            && !self.loading_completed
        {
            self.loading_completed = true;
            Some(Effect::Session(SessionEffect::LoadCompleted))
        } else {
            None
        }
    }

    pub(crate) fn update(&mut self, input: Input) -> Option<Effect> {
        if let Some(overlay) = self.overlay {
            let Input::Menu(key) = input else {
                return None;
            };
            return self.update_overlay(overlay, key);
        }
        match input {
            Input::Action(action) => self.action(action),
            Input::Text(edit) => self.edit_text(edit),
            Input::Menu(_) => None,
        }
    }

    fn edit_text(&mut self, edit: TextEdit) -> Option<Effect> {
        match self.mode {
            InputState::Filter => {
                if self.query.edit(edit) {
                    return self.query_changed();
                }
                None
            }
            InputState::Capture => {
                if self.capture.edit(edit) {
                    self.capture_changed();
                }
                None
            }
            InputState::Browse => None,
        }
    }

    fn query_changed(&mut self) -> Option<Effect> {
        self.cursor = 0;
        self.status = None;
        self.refilter()
    }

    fn capture_changed(&mut self) {
        self.status = None;
        self.preview_add = self.parse_capture();
    }

    fn parse_capture(&self) -> Option<Result<NewReminder, String>> {
        let text = self.capture.text.trim();
        (!text.is_empty()).then(|| {
            let mut new = crate::cli::quick_add(text, &self.now, &self.config)
                .map_err(|error| error.to_string())?;
            if let CapturePriority::Set(priority) = self.capture_priority {
                new.priority = priority;
            }
            Ok(new)
        })
    }

    fn action(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Quit => {
                self.quit = true;
                return None;
            }
            Action::Help => {
                self.help_scroll = 0;
                self.overlay = Some(Overlay::Help);
                return None;
            }
            Action::PreviewToggle => {
                self.preview = !self.preview;
                return None;
            }
            Action::None => return None,
            _ => {}
        }
        match self.mode {
            InputState::Browse => self.browse(action),
            InputState::Filter => self.filtering(action),
            InputState::Capture => self.capturing(action),
        }
    }

    fn browse(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Filter => {
                self.mode = InputState::Filter;
                let mut text = self.applied.clone();
                if !text.is_empty() {
                    text.push(' ');
                }
                self.query.set(text);
                self.focus = Pane::Items;
                self.query_changed()
            }
            Action::Capture | Action::Add => {
                self.mode = InputState::Capture;
                self.capture = Line::default();
                self.capture_priority = CapturePriority::Parsed;
                self.preview_add = None;
                self.status = None;
                None
            }
            Action::ClearFilter => {
                self.applied.clear();
                self.query_changed()
            }
            Action::Accept => {
                if self.focus == Pane::Lists {
                    self.focus = Pane::Items;
                }
                None
            }
            Action::Cancel => {
                self.selected.clear();
                self.status = None;
                None
            }
            Action::KeepFilter => None,
            _ => self.common(action),
        }
    }

    fn filtering(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::ClearFilter => {
                self.query = Line::default();
                self.query_changed()
            }
            Action::Accept if self.filter_error.is_some() => None,
            Action::Accept if matches!(self.current(), Some(Row::Add(_))) => self.add_from_query(),
            Action::Add => self.add_from_query(),
            Action::Accept | Action::KeepFilter => {
                if self.filter_error.is_some() {
                    return None;
                }
                self.applied = self.query.text.trim().to_owned();
                self.leave_filter()
            }
            Action::Cancel => {
                self.applied.clear();
                self.leave_filter()
            }
            Action::Filter | Action::Capture => None,
            _ => self.common(action),
        }
    }

    fn leave_filter(&mut self) -> Option<Effect> {
        self.query = Line::default();
        self.mode = InputState::Browse;
        self.query_changed()
    }

    fn add_from_query(&mut self) -> Option<Effect> {
        let text = self.add_text()?.to_owned();
        Some(Effect::Session(SessionEffect::Add(text)))
    }

    fn capturing(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Accept => match self.parse_capture() {
                Some(Ok(new)) => Some(Effect::Session(SessionEffect::AddParsed(new))),
                Some(Err(error)) => {
                    self.set_status(&error, true);
                    None
                }
                None => {
                    self.set_status("Type a reminder title first.", true);
                    None
                }
            },
            Action::Cancel => {
                self.leave_capture();
                None
            }
            Action::PriorityHigh => self.capture_priority(Some(Priority::High)),
            Action::PriorityMedium => self.capture_priority(Some(Priority::Medium)),
            Action::PriorityLow => self.capture_priority(Some(Priority::Low)),
            Action::PriorityNone => self.capture_priority(None),
            Action::Up | Action::Down | Action::PageUp | Action::PageDown => {
                self.move_items(action);
                None
            }
            _ => None,
        }
    }

    fn capture_priority(&mut self, priority: Option<Priority>) -> Option<Effect> {
        self.capture_priority = CapturePriority::Set(priority);
        self.capture_changed();
        None
    }

    fn leave_capture(&mut self) {
        self.mode = InputState::Browse;
        self.capture = Line::default();
        self.capture_priority = CapturePriority::Parsed;
        self.preview_add = None;
    }

    fn common(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Up
            | Action::Down
            | Action::PageUp
            | Action::PageDown
            | Action::Top
            | Action::Bottom => {
                if self.focus == Pane::Lists {
                    return self.move_scope(action);
                }
                self.move_items(action);
                None
            }
            Action::NextPane | Action::PrevPane => {
                self.focus = match self.focus {
                    Pane::Lists => Pane::Items,
                    Pane::Items => Pane::Lists,
                };
                None
            }
            Action::SelectToggle => {
                if let Some(id) = self.current_reminder().map(|item| item.id.clone()) {
                    if let Some(position) = self.selected.iter().position(|item| *item == id) {
                        self.selected.remove(position);
                    } else {
                        self.selected.push(id);
                    }
                }
                None
            }
            Action::SelectAll => {
                let visible: Vec<String> = self
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        RowIndex::Reminder(index) => Some(self.data.items[*index].id.clone()),
                        RowIndex::Event(_) => None,
                    })
                    .collect();
                if !visible.is_empty() && visible.iter().all(|id| self.is_selected(id)) {
                    self.selected.retain(|id| !visible.contains(id));
                } else {
                    for id in visible {
                        if !self.is_selected(&id) {
                            self.selected.push(id);
                        }
                    }
                }
                None
            }
            Action::ActMenu => {
                if self.take_targets() {
                    self.open(Overlay::Act);
                }
                None
            }
            Action::Done => self.change(TaskChange::Complete),
            Action::Reopen => self.change(TaskChange::Reopen),
            Action::PriorityHigh => self.change(TaskChange::Priority(Some(Priority::High))),
            Action::PriorityMedium => self.change(TaskChange::Priority(Some(Priority::Medium))),
            Action::PriorityLow => self.change(TaskChange::Priority(Some(Priority::Low))),
            Action::PriorityNone => self.change(TaskChange::Priority(None)),
            Action::Move => {
                if self.take_targets() {
                    self.open_move();
                }
                None
            }
            Action::Edit => {
                if self.take_targets() {
                    return Some(Effect::Edit(self.targets.clone()));
                }
                None
            }
            Action::Delete => {
                if self.take_targets() {
                    self.open_delete();
                }
                None
            }
            _ => None,
        }
    }

    fn change(&mut self, change: TaskChange) -> Option<Effect> {
        if !self.take_targets() {
            return None;
        }
        Some(Effect::Session(SessionEffect::Change {
            change,
            ids: self.targets.clone(),
        }))
    }

    fn take_targets(&mut self) -> bool {
        if !self.selected.is_empty() {
            self.targets = self.selected.clone();
            self.status = None;
            return true;
        }
        match self.current() {
            Some(Row::Reminder(item)) => {
                self.targets = vec![item.id.clone()];
                self.status = None;
                true
            }
            Some(Row::Event(_)) => {
                self.set_status("Events are read-only.", true);
                false
            }
            _ => {
                self.set_status("Nothing to act on.", true);
                false
            }
        }
    }

    fn open(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.menu_cursor = 0;
    }

    fn open_move(&mut self) {
        if self.data.lists.is_empty() {
            self.overlay = None;
            self.set_status("No writable reminder lists.", true);
        } else {
            self.open(Overlay::Move);
        }
    }

    fn open_delete(&mut self) {
        self.open(Overlay::ConfirmDelete);
        self.confirm_input.clear();
    }

    fn move_items(&mut self, action: Action) {
        let last = self.row_count().saturating_sub(1);
        self.cursor = step(self.cursor, last, action);
    }

    fn move_scope(&mut self, action: Action) -> Option<Effect> {
        let mut scopes = self.scopes();
        let index = self.scope_cursor();
        let next = step(index, scopes.len().saturating_sub(1), action);
        if next == index {
            return None;
        }
        self.scope = scopes.swap_remove(next);
        self.query_changed()
    }

    fn update_overlay(&mut self, overlay: Overlay, key: MenuKey) -> Option<Effect> {
        match overlay {
            Overlay::Help => {
                match key {
                    MenuKey::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                    MenuKey::Down => {
                        self.help_scroll = (self.help_scroll + 1).min(self.help_limit.get());
                    }
                    _ => self.overlay = None,
                }
                None
            }
            Overlay::Act => {
                let choice = self.menu_choice(key, ACTS.len(), |key| {
                    ACTS.iter().position(|(hotkey, _)| *hotkey == key)
                })?;
                self.overlay = None;
                match ACTS[choice].1 {
                    "done" => Some(self.retarget(TaskChange::Complete)),
                    "reopen" => Some(self.retarget(TaskChange::Reopen)),
                    "priority" => {
                        self.open(Overlay::Priority);
                        None
                    }
                    "move" => {
                        self.open_move();
                        None
                    }
                    "edit" => Some(Effect::Edit(self.targets.clone())),
                    _ => {
                        self.open_delete();
                        None
                    }
                }
            }
            Overlay::Priority => {
                let choice = self.menu_choice(key, PRIORITIES.len(), |key| {
                    PRIORITIES.iter().position(|(hotkey, ..)| *hotkey == key)
                })?;
                self.overlay = None;
                Some(self.retarget(TaskChange::Priority(PRIORITIES[choice].2)))
            }
            Overlay::Move => {
                let count = self.data.lists.len();
                let choice = self.menu_choice(key, count, |key| {
                    key.to_digit(10)
                        .and_then(|digit| usize::try_from(digit).ok())
                        .filter(|digit| (1..=count).contains(digit))
                        .map(|digit| digit - 1)
                })?;
                self.overlay = None;
                Some(self.retarget(TaskChange::Move(self.data.lists[choice].id.clone())))
            }
            Overlay::ConfirmDelete => {
                let key = match key {
                    MenuKey::Char(character) => ConfirmKey::Char(character),
                    MenuKey::Backspace => ConfirmKey::Backspace,
                    MenuKey::Accept => ConfirmKey::Accept,
                    MenuKey::Cancel => ConfirmKey::Cancel,
                    MenuKey::Up | MenuKey::Down => ConfirmKey::Other,
                };
                let step = confirm_delete(
                    self.delete_needs_count(),
                    self.partial_target_count(),
                    self.targets.len(),
                    &mut self.confirm_input,
                    key,
                );
                match step {
                    ConfirmStep::Stay => None,
                    ConfirmStep::Delete { partial_confirmed } => {
                        self.overlay = None;
                        Some(Effect::Session(SessionEffect::Delete {
                            ids: self.targets.clone(),
                            partial_confirmed,
                        }))
                    }
                    ConfirmStep::Cancelled { said } => {
                        self.overlay = None;
                        if said {
                            self.set_status(NOTHING_DELETED, false);
                        }
                        None
                    }
                }
            }
        }
    }

    fn retarget(&self, change: TaskChange) -> Effect {
        Effect::Session(SessionEffect::Change {
            change,
            ids: self.targets.clone(),
        })
    }

    fn menu_choice(
        &mut self,
        key: MenuKey,
        len: usize,
        hotkey: impl Fn(char) -> Option<usize>,
    ) -> Option<usize> {
        match key {
            MenuKey::Up => {
                self.menu_cursor = self.menu_cursor.checked_sub(1).unwrap_or(len - 1);
                None
            }
            MenuKey::Down => {
                self.menu_cursor = (self.menu_cursor + 1) % len;
                None
            }
            MenuKey::Accept => Some(self.menu_cursor),
            MenuKey::Char(key) => hotkey(key),
            MenuKey::Cancel => {
                self.overlay = None;
                None
            }
            MenuKey::Backspace => None,
        }
    }

    pub(crate) fn resolve(&mut self, response: Response) -> Option<Effect> {
        match response {
            Response::Loaded(data) => {
                self.loading_completed = false;
                self.replace_data(data);
                self.refilter()
            }
            Response::Added(item) => {
                let title = item.title.clone();
                let id = item.id.clone();
                self.data.items.push(item);
                if self.mode == InputState::Capture {
                    self.leave_capture();
                } else if self.mode == InputState::Filter {
                    self.applied.clear();
                    self.query = Line::default();
                    self.mode = InputState::Browse;
                }
                let effect = self.refilter();
                let shown = self.rows.iter().position(|row| match row {
                    RowIndex::Reminder(index) => self.data.items[*index].id == id,
                    RowIndex::Event(_) => false,
                });
                if let Some(position) = shown {
                    self.cursor = position;
                    self.focus = Pane::Items;
                }
                let hidden = if shown.is_some() {
                    ""
                } else {
                    " (hidden by the current list or filter)"
                };
                self.set_status(
                    &format!("Added \"{}\"{hidden}", crate::cli::output::clean(&title)),
                    false,
                );
                effect
            }
            Response::Changed {
                message,
                error,
                data,
            } => {
                self.overlay = None;
                self.set_status(&message, error);
                let data = data?;
                if !error {
                    self.selected.clear();
                    self.targets.clear();
                }
                self.replace_data(data);
                self.refilter()
            }
            Response::DeleteRefused {
                message,
                partial,
                data,
            } => {
                if let Some(data) = data {
                    self.replace_data(data);
                }
                for item in &mut self.data.items {
                    if partial.contains(&item.id) {
                        item.backup_partial = true;
                    }
                }
                self.overlay = Some(Overlay::ConfirmDelete);
                self.confirm_input.clear();
                self.set_status(&message, true);
                self.refilter()
            }
            Response::Failed(message) => {
                self.loading_completed = false;
                self.overlay = None;
                self.set_status(&message, true);
                None
            }
        }
    }

    fn replace_data(&mut self, data: Data) {
        let index = self.scope_cursor();
        self.data = data;
        let mut scopes = self.scopes();
        if !scopes.contains(&self.scope) {
            self.scope = scopes.swap_remove(index.min(scopes.len() - 1));
        }
    }

    fn set_status(&mut self, text: &str, error: bool) {
        self.status = Some(Status {
            text: text.to_owned(),
            error,
        });
    }
}

fn step(position: usize, last: usize, action: Action) -> usize {
    match action {
        Action::Up => position.saturating_sub(1),
        Action::Down => (position + 1).min(last),
        Action::PageUp => position.saturating_sub(PAGE),
        Action::PageDown => (position + PAGE).min(last),
        Action::Top => 0,
        Action::Bottom => last,
        _ => position,
    }
}

fn reminder_row(item: &Item) -> FilterRow<'_> {
    FilterRow {
        kind: FilterKind::Reminder,
        title: &item.title,
        collection: item.list.as_deref().unwrap_or_default(),
        due: item.due,
        due_time: item.due_time,
        due_at: item.due_at,
        priority: item.priority,
        completed: item.completed,
    }
}

pub(crate) fn local_start(event: &Event, now: &Zoned) -> Zoned {
    event.start.with_time_zone(now.time_zone().clone())
}

fn event_row<'a>(event: &'a Event, now: &Zoned) -> FilterRow<'a> {
    FilterRow {
        kind: FilterKind::Event,
        title: &event.title,
        collection: &event.calendar,
        due: Some(if event.all_day {
            event.start.date()
        } else {
            local_start(event, now).date()
        }),
        due_time: None,
        due_at: None,
        priority: None,
        completed: false,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::tui::state::tests::{data, item, now};

    pub(crate) fn events() -> Vec<Event> {
        let zone = TimeZone::fixed(offset(-4));
        let at = |day: i8, hour: i8| {
            date(2026, 10, day)
                .at(hour, 0, 0, 0)
                .to_zoned(zone.clone())
                .unwrap()
        };
        vec![
            Event {
                id: "event-2".into(),
                title: "Lunch with Sam".into(),
                start: at(31, 12),
                all_day: false,
                calendar: "Personal".into(),
                location: Some("Cafe Rio".into()),
                notes: None,
                meeting_url: None,
            },
            Event {
                id: "event-1".into(),
                title: "Standup".into(),
                start: at(30, 9),
                all_day: false,
                calendar: "Work".into(),
                location: None,
                notes: Some("Daily check-in.".into()),
                meeting_url: Some("https://zoom.us/j/1".into()),
            },
        ]
    }

    pub(crate) fn board_with(data: Data, config: Config) -> Board {
        let (board, effect) = Board::new(data, events(), config, now(), None);
        assert_eq!(effect, None);
        board
    }

    fn board() -> Board {
        board_with(data(), Config::default())
    }

    fn act(board: &mut Board, action: Action) -> Option<Effect> {
        board.update(Input::Action(action))
    }

    fn menu(board: &mut Board, key: MenuKey) -> Option<Effect> {
        board.update(Input::Menu(key))
    }

    fn type_text(board: &mut Board, text: &str) -> Option<Effect> {
        let mut effect = None;
        for character in text.chars() {
            effect = board
                .update(Input::Text(TextEdit::Insert(character)))
                .or(effect);
        }
        effect
    }

    fn ids(board: &Board) -> Vec<String> {
        board
            .rows()
            .iter()
            .map(|row| match row {
                Row::Reminder(item) => item.id.clone(),
                Row::Event(event) => event.id.clone(),
                Row::Add(text) => format!("add:{text}"),
            })
            .collect()
    }

    fn scope_names(board: &Board) -> Vec<String> {
        board
            .scopes()
            .iter()
            .map(|scope| board.scope_name(scope))
            .collect()
    }

    fn change(change: TaskChange, ids: &[&str]) -> Effect {
        Effect::Session(SessionEffect::Change {
            change,
            ids: ids.iter().map(ToString::to_string).collect(),
        })
    }

    #[test]
    fn the_board_starts_in_browse_with_reminders_then_events() {
        let board = board();
        assert_eq!(board.mode(), InputState::Browse);
        assert_eq!(board.focus(), Pane::Items);
        assert_eq!(
            ids(&board),
            ["rem-1", "rem-2", "rem-3", "event-1", "event-2"]
        );
        assert_eq!(
            scope_names(&board),
            ["All", "Today", "Home", "Work", "Events"]
        );
        assert_eq!(board.scope_count(&Scope::Today), 2);
    }

    #[test]
    fn saved_views_follow_the_lists_and_combine_with_the_filter() {
        let mut config = Config::default();
        config
            .tui
            .views
            .insert("Work, high".into(), "list:work pri:medium".into());
        config.tui.views.insert("Soon".into(), "due:week".into());
        let mut board = board_with(data(), config);
        assert_eq!(
            scope_names(&board),
            [
                "All",
                "Today",
                "Home",
                "Work",
                "Events",
                "Soon",
                "Work, high"
            ]
        );
        act(&mut board, Action::PrevPane);
        act(&mut board, Action::Bottom);
        assert_eq!(board.current_scope(), Scope::View(1));
        assert_eq!(ids(&board), ["rem-3"]);
        act(&mut board, Action::Up);
        assert_eq!(ids(&board), ["rem-1", "rem-3", "event-1", "event-2"]);
        act(&mut board, Action::Filter);
        type_text(&mut board, "file");
        assert_eq!(ids(&board), ["rem-3", "add:file"]);
    }

    #[test]
    fn list_scopes_match_the_list_exactly_and_hide_events() {
        let mut board = board();
        act(&mut board, Action::PrevPane);
        assert_eq!(board.focus(), Pane::Lists);
        act(&mut board, Action::Down);
        assert_eq!(ids(&board), ["rem-1", "event-1"]);
        act(&mut board, Action::Down);
        assert_eq!(board.current_scope(), Scope::List("Home".into()));
        assert_eq!(ids(&board), ["rem-1"]);
        act(&mut board, Action::Down);
        act(&mut board, Action::Down);
        assert_eq!(board.current_scope(), Scope::Events);
        assert_eq!(ids(&board), ["event-1", "event-2"]);
        act(&mut board, Action::Accept);
        assert_eq!(board.focus(), Pane::Items);
    }

    #[test]
    fn slash_filters_enter_keeps_and_escape_clears() {
        let mut board = board();
        act(&mut board, Action::Filter);
        assert_eq!(board.mode(), InputState::Filter);
        type_text(&mut board, "buy");
        assert_eq!(ids(&board), ["rem-1", "rem-2", "add:buy"]);
        act(&mut board, Action::Accept);
        assert_eq!(board.mode(), InputState::Browse);
        assert_eq!(board.applied(), "buy");
        assert_eq!(ids(&board), ["rem-1", "rem-2"]);

        act(&mut board, Action::Filter);
        assert_eq!(board.query().text, "buy ");
        type_text(&mut board, "eggs");
        assert_eq!(ids(&board), ["rem-2", "add:buy eggs"]);
        act(&mut board, Action::Cancel);
        assert_eq!(board.mode(), InputState::Browse);
        assert_eq!(board.applied(), "");
        assert_eq!(ids(&board).len(), 5);
    }

    #[test]
    fn an_invalid_filter_keeps_the_last_results_and_ignores_enter() {
        let mut board = board();
        act(&mut board, Action::Filter);
        type_text(&mut board, "pri:high");
        board.update(Input::Text(TextEdit::Backspace));
        assert_eq!(
            board.filter_error(),
            Some("invalid value \"hig\" for pri filter; expected high, medium, low, or none")
        );
        assert_eq!(ids(&board), ["rem-1"]);
        assert_eq!(act(&mut board, Action::Accept), None);
        assert_eq!(board.mode(), InputState::Filter);
        type_text(&mut board, "h");
        assert_eq!(ids(&board), ["rem-1"]);
    }

    #[test]
    fn the_add_row_adds_the_typed_text_and_returns_to_browse() {
        let mut board = board();
        act(&mut board, Action::Filter);
        type_text(&mut board, "Call mom fri !high");
        act(&mut board, Action::Bottom);
        assert_eq!(
            act(&mut board, Action::Accept),
            Some(Effect::Session(SessionEffect::Add(
                "Call mom fri !high".into()
            )))
        );
        let added = item("rem-9", "Call mom", "Home", None, Some(Priority::High));
        assert_eq!(board.resolve(Response::Added(added)), None);
        assert_eq!(board.mode(), InputState::Browse);
        assert_eq!(board.applied(), "");
        assert_eq!(
            board
                .current()
                .map(|row| matches!(row, Row::Reminder(item) if item.id == "rem-9")),
            Some(true)
        );
        assert_eq!(
            board.status().map(|status| status.text.as_str()),
            Some("Added \"Call mom\"")
        );
    }

    #[test]
    fn capture_previews_the_quick_add_parse_and_adds_on_enter() {
        let mut board = board();
        act(&mut board, Action::Capture);
        assert_eq!(board.mode(), InputState::Capture);
        type_text(&mut board, "Pay rent tomorrow #Work");
        let Some(Ok(preview)) = board.capture_preview() else {
            panic!("expected a preview");
        };
        assert_eq!(preview.title, "Pay rent");
        assert_eq!(preview.due, Some(date(2026, 10, 31)));
        assert_eq!(preview.list.as_deref(), Some("Work"));
        act(&mut board, Action::PriorityHigh);
        act(&mut board, Action::PriorityLow);
        assert_eq!(board.capture().text, "Pay rent tomorrow #Work");
        let Some(Effect::Session(SessionEffect::AddParsed(new))) = act(&mut board, Action::Accept)
        else {
            panic!("expected a parsed reminder to add");
        };
        assert_eq!(new.title, "Pay rent");
        assert_eq!(new.due, Some(date(2026, 10, 31)));
        assert_eq!(new.list.as_deref(), Some("Work"));
        assert_eq!(new.priority, Some(Priority::Low));
        board.resolve(Response::Failed("no list named Work".into()));
        assert_eq!(board.mode(), InputState::Capture);
        assert_eq!(board.status().map(|status| status.error), Some(true));
        act(&mut board, Action::Cancel);
        assert_eq!(board.mode(), InputState::Browse);
        assert_eq!(board.capture().text, "");
    }

    fn captured(board: &mut Board) -> NewReminder {
        match act(board, Action::Accept) {
            Some(Effect::Session(SessionEffect::AddParsed(new))) => new,
            other => panic!("expected a parsed reminder to add, got {other:?}"),
        }
    }

    #[test]
    fn capture_priority_keys_override_the_parsed_priority_without_editing_text() {
        let mut board = board();
        act(&mut board, Action::Capture);
        type_text(&mut board, "Discuss !high option !low");
        act(&mut board, Action::PriorityMedium);
        assert_eq!(board.capture().text, "Discuss !high option !low");
        let new = captured(&mut board);
        assert_eq!(new.title, "Discuss !high option");
        assert_eq!(new.priority, Some(Priority::Medium));

        let mut config = Config::default();
        config.task.parse_inline = false;
        config.task.priority = Some(Priority::Low);
        let mut board = board_with(data(), config);
        act(&mut board, Action::Capture);
        type_text(&mut board, "Ship !high");
        assert_eq!(
            board.capture_preview().unwrap().as_ref().unwrap().priority,
            Some(Priority::Low)
        );
        act(&mut board, Action::PriorityHigh);
        let new = captured(&mut board);
        assert_eq!(new.title, "Ship !high");
        assert_eq!(new.priority, Some(Priority::High));
        act(&mut board, Action::PriorityNone);
        assert_eq!(captured(&mut board).priority, None);

        act(&mut board, Action::Cancel);
        act(&mut board, Action::Capture);
        type_text(&mut board, "Ship");
        assert_eq!(captured(&mut board).priority, Some(Priority::Low));
    }

    #[test]
    fn capture_reports_parse_errors_and_refuses_a_blank_title() {
        let mut board = board();
        act(&mut board, Action::Capture);
        assert_eq!(act(&mut board, Action::Accept), None);
        assert_eq!(board.status().map(|status| status.error), Some(true));
        type_text(&mut board, "Pay !low !high");
        assert!(
            matches!(board.capture_preview(), Some(Err(error)) if error.contains("more than once"))
        );
        assert_eq!(act(&mut board, Action::Accept), None);
        assert!(
            board
                .status()
                .is_some_and(|status| status.error && status.text.contains("more than once"))
        );
    }

    #[test]
    fn an_added_reminder_shows_in_its_list() {
        let mut board = board();
        act(&mut board, Action::PrevPane);
        act(&mut board, Action::Down);
        act(&mut board, Action::Down);
        assert_eq!(board.current_scope(), Scope::List("Home".into()));
        act(&mut board, Action::Capture);
        type_text(&mut board, "Water plants");
        captured(&mut board);
        let added = item("rem-9", "Water plants", "Home", None, None);
        assert_eq!(board.resolve(Response::Added(added)), None);
        assert_eq!(ids(&board), ["rem-1", "rem-9"]);
        assert_eq!(
            board.status().map(|status| status.text.as_str()),
            Some("Added \"Water plants\"")
        );
    }

    #[test]
    fn a_failed_change_keeps_the_selection() {
        let mut board = board();
        act(&mut board, Action::SelectAll);
        act(&mut board, Action::Done);
        board.resolve(Response::Changed {
            message: "the editor failed".into(),
            error: true,
            data: Some(data()),
        });
        assert_eq!(board.selected(), ["rem-1", "rem-2", "rem-3"]);
        assert_eq!(board.targets(), ["rem-1", "rem-2", "rem-3"]);
        board.resolve(Response::Changed {
            message: "Couldn't reload reminders".into(),
            error: false,
            data: None,
        });
        assert_eq!(board.selected(), ["rem-1", "rem-2", "rem-3"]);
        board.resolve(Response::Changed {
            message: "Applied 3 changes.".into(),
            error: false,
            data: Some(data()),
        });
        assert_eq!(board.selected(), Vec::<String>::new());
        assert_eq!(board.targets(), Vec::<String>::new());
    }

    #[test]
    fn the_highlighted_list_survives_reloads_that_add_or_remove_lists() {
        let mut config = Config::default();
        config.tui.views.insert("Soon".into(), "due:week".into());
        let mut board = board_with(data(), config);
        act(&mut board, Action::PrevPane);
        act(&mut board, Action::Bottom);
        assert_eq!(board.current_scope(), Scope::View(0));

        let mut more = data();
        more.items
            .push(item("rem-8", "Old task", "Archive", None, None));
        board.resolve(Response::Loaded(more));
        assert_eq!(board.current_scope(), Scope::View(0));
        assert_eq!(board.scope_cursor(), 6);

        act(&mut board, Action::Top);
        act(&mut board, Action::Down);
        act(&mut board, Action::Down);
        act(&mut board, Action::Down);
        assert_eq!(board.current_scope(), Scope::List("Work".into()));
        let mut fewer = data();
        fewer.lists.remove(0);
        fewer.items.remove(0);
        board.resolve(Response::Loaded(fewer));
        assert_eq!(board.current_scope(), Scope::List("Work".into()));
        assert_eq!(ids(&board), ["rem-2", "rem-3"]);

        let mut gone = data();
        gone.lists.remove(1);
        gone.items.truncate(1);
        board.resolve(Response::Loaded(gone));
        assert_eq!(board.current_scope(), Scope::List("Home".into()));
    }

    #[test]
    fn help_scroll_stops_at_the_rendered_limit() {
        let mut board = board();
        act(&mut board, Action::Help);
        board.set_help_limit(2);
        for _ in 0..10 {
            menu(&mut board, MenuKey::Down);
        }
        assert_eq!(board.help_scroll(), 2);
        menu(&mut board, MenuKey::Up);
        assert_eq!(board.help_scroll(), 1);
    }

    #[test]
    fn one_step_actions_target_the_highlight_or_the_selection() {
        let mut board = board();
        assert_eq!(
            act(&mut board, Action::Done),
            Some(change(TaskChange::Complete, &["rem-1"]))
        );
        act(&mut board, Action::SelectToggle);
        act(&mut board, Action::Down);
        act(&mut board, Action::SelectToggle);
        assert_eq!(board.selected(), ["rem-1", "rem-2"]);
        assert_eq!(
            act(&mut board, Action::PriorityLow),
            Some(change(
                TaskChange::Priority(Some(Priority::Low)),
                &["rem-1", "rem-2"]
            ))
        );
        assert_eq!(
            act(&mut board, Action::Reopen),
            Some(change(TaskChange::Reopen, &["rem-1", "rem-2"]))
        );
        assert_eq!(
            act(&mut board, Action::Edit),
            Some(Effect::Edit(vec!["rem-1".into(), "rem-2".into()]))
        );
        board.resolve(Response::Changed {
            message: "Applied 1 change: 1 updated.".into(),
            error: false,
            data: Some(data()),
        });
        assert_eq!(board.selected(), Vec::<String>::new());
    }

    #[test]
    fn events_are_read_only() {
        let mut board = board();
        act(&mut board, Action::Bottom);
        assert!(matches!(board.current(), Some(Row::Event(_))));
        for action in [
            Action::Done,
            Action::Edit,
            Action::Delete,
            Action::ActMenu,
            Action::Move,
        ] {
            assert_eq!(act(&mut board, action), None);
            assert_eq!(board.overlay(), None);
            assert_eq!(
                board.status().map(|status| status.text.as_str()),
                Some("Events are read-only.")
            );
        }
        act(&mut board, Action::SelectToggle);
        assert_eq!(board.selected(), Vec::<String>::new());
    }

    #[test]
    fn select_all_toggles_the_visible_reminders() {
        let mut board = board();
        act(&mut board, Action::SelectAll);
        assert_eq!(board.selected(), ["rem-1", "rem-2", "rem-3"]);
        act(&mut board, Action::SelectAll);
        assert_eq!(board.selected(), Vec::<String>::new());
    }

    #[test]
    fn the_act_menu_offers_done_reopen_priority_move_edit_and_delete() {
        let mut board = board();
        act(&mut board, Action::ActMenu);
        assert_eq!(board.overlay(), Some(Overlay::Act));
        assert_eq!(
            menu(&mut board, MenuKey::Char('r')),
            Some(change(TaskChange::Reopen, &["rem-1"]))
        );
        act(&mut board, Action::ActMenu);
        menu(&mut board, MenuKey::Char('p'));
        assert_eq!(board.overlay(), Some(Overlay::Priority));
        assert_eq!(
            menu(&mut board, MenuKey::Char('n')),
            Some(change(TaskChange::Priority(None), &["rem-1"]))
        );
        act(&mut board, Action::ActMenu);
        menu(&mut board, MenuKey::Char('m'));
        assert_eq!(board.overlay(), Some(Overlay::Move));
        assert_eq!(
            menu(&mut board, MenuKey::Char('2')),
            Some(change(TaskChange::Move("list-work".into()), &["rem-1"]))
        );
        act(&mut board, Action::ActMenu);
        menu(&mut board, MenuKey::Down);
        menu(&mut board, MenuKey::Down);
        menu(&mut board, MenuKey::Down);
        menu(&mut board, MenuKey::Down);
        assert_eq!(
            menu(&mut board, MenuKey::Accept),
            Some(Effect::Edit(vec!["rem-1".into()]))
        );
        act(&mut board, Action::ActMenu);
        menu(&mut board, MenuKey::Cancel);
        assert_eq!(board.overlay(), None);
    }

    #[test]
    fn delete_asks_with_y_for_a_few_fully_backed_up_reminders() {
        let mut board = board();
        act(&mut board, Action::Delete);
        assert_eq!(board.overlay(), Some(Overlay::ConfirmDelete));
        assert!(!board.delete_needs_count());
        assert_eq!(menu(&mut board, MenuKey::Accept), None);
        assert_eq!(menu(&mut board, MenuKey::Char('n')), None);
        assert_eq!(board.overlay(), None);
        act(&mut board, Action::Delete);
        assert_eq!(
            menu(&mut board, MenuKey::Char('y')),
            Some(Effect::Session(SessionEffect::Delete {
                ids: vec!["rem-1".into()],
                partial_confirmed: false,
            }))
        );
    }

    #[test]
    fn deleting_more_than_three_or_partial_backups_needs_the_typed_count() {
        let mut more = data();
        more.items
            .push(item("rem-4", "Water plants", "Home", None, None));
        let mut board = board_with(more, Config::default());
        act(&mut board, Action::SelectAll);
        act(&mut board, Action::Delete);
        assert!(board.delete_needs_count());
        assert_eq!(menu(&mut board, MenuKey::Char('y')), None);
        assert_eq!(board.overlay(), None);
        assert_eq!(
            board.status().map(|status| status.text.as_str()),
            Some(NOTHING_DELETED)
        );
        act(&mut board, Action::Delete);
        menu(&mut board, MenuKey::Char('4'));
        assert_eq!(
            menu(&mut board, MenuKey::Accept),
            Some(Effect::Session(SessionEffect::Delete {
                ids: vec![
                    "rem-1".into(),
                    "rem-2".into(),
                    "rem-3".into(),
                    "rem-4".into()
                ],
                partial_confirmed: false,
            }))
        );

        let mut partial = data();
        partial.items[0].backup_partial = true;
        let mut board = board_with(partial, Config::default());
        act(&mut board, Action::Delete);
        assert!(board.delete_needs_count());
        menu(&mut board, MenuKey::Char('1'));
        assert_eq!(
            menu(&mut board, MenuKey::Accept),
            Some(Effect::Session(SessionEffect::Delete {
                ids: vec!["rem-1".into()],
                partial_confirmed: true,
            }))
        );
    }

    #[test]
    fn a_refused_delete_reopens_the_confirmation_and_needs_the_count() {
        let mut board = board();
        act(&mut board, Action::Delete);
        menu(&mut board, MenuKey::Char('y'));
        board.resolve(Response::DeleteRefused {
            message: "refused".into(),
            partial: vec!["rem-1".into()],
            data: None,
        });
        assert_eq!(board.overlay(), Some(Overlay::ConfirmDelete));
        assert!(board.delete_needs_count());
        assert_eq!(menu(&mut board, MenuKey::Char('y')), None);
        assert_eq!(board.overlay(), None);
    }

    #[test]
    fn is_done_loads_completed_reminders_once() {
        let mut board = board();
        act(&mut board, Action::Filter);
        assert_eq!(
            type_text(&mut board, "is:done"),
            Some(Effect::Session(SessionEffect::LoadCompleted))
        );
        assert_eq!(type_text(&mut board, " "), None);
        let mut completed = data();
        completed.includes_completed = true;
        let mut done = item("rem-7", "Call the dentist", "Home", None, None);
        done.completed = true;
        completed.items.push(done);
        assert_eq!(board.resolve(Response::Loaded(completed)), None);
        assert_eq!(ids(&board), ["rem-7"]);
    }

    #[test]
    fn help_quit_and_preview_are_available_everywhere() {
        let mut board = board();
        act(&mut board, Action::PreviewToggle);
        assert!(!board.preview());
        act(&mut board, Action::Capture);
        act(&mut board, Action::Help);
        assert_eq!(board.overlay(), Some(Overlay::Help));
        menu(&mut board, MenuKey::Down);
        assert_eq!(board.help_scroll(), 1);
        menu(&mut board, MenuKey::Char('q'));
        assert_eq!(board.overlay(), None);
        assert_eq!(board.mode(), InputState::Capture);
        act(&mut board, Action::Quit);
        assert!(board.quit());
    }

    #[test]
    fn moving_stays_within_the_rows() {
        let mut board = board();
        act(&mut board, Action::PageDown);
        assert_eq!(board.cursor(), 4);
        act(&mut board, Action::Top);
        assert_eq!(board.cursor(), 0);
        act(&mut board, Action::Up);
        assert_eq!(board.cursor(), 0);
    }
}

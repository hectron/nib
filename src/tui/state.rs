use jiff::civil::{Date, Time};
use jiff::{Timestamp, Zoned};

use crate::app::filter::{Filter, FilterKind, FilterRow};
use crate::domain::{Priority, TaskChange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: String,
    pub title: String,
    pub list: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub due: Option<Date>,
    pub due_time: Option<Time>,
    pub due_at: Option<Timestamp>,
    pub priority: Option<Priority>,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListChoice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Data {
    pub items: Vec<Item>,
    pub lists: Vec<ListChoice>,
    pub includes_completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Filter,
    Act,
    Priority,
    Move,
    ConfirmDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Insert(char),
    Backspace,
    DeleteForward,
    CaretLeft,
    CaretRight,
    CaretStart,
    CaretEnd,
    KillToStart,
    KillToEnd,
    KillWord,
    Up,
    Down,
    ToggleDown,
    ToggleUp,
    Accept,
    OpenMenu,
    Hotkey(char),
    Cancel,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    LoadCompleted,
    Add(String),
    Change {
        change: TaskChange,
        ids: Vec<String>,
    },
    Delete(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Response {
    Loaded(Data),
    Added(Item),
    Changed { message: String, data: Data },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Picked(Vec<String>),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row<'a> {
    Item(&'a Item),
    Add(&'a str),
}

pub(crate) const ACTS: [(char, &str); 4] = [
    ('x', "done"),
    ('p', "priority"),
    ('m', "move"),
    ('d', "delete"),
];

pub(crate) const PRIORITIES: [(char, &str, Option<Priority>); 4] = [
    ('h', "high", Some(Priority::High)),
    ('m', "medium", Some(Priority::Medium)),
    ('l', "low", Some(Priority::Low)),
    ('n', "none", None),
];

pub(crate) struct State {
    now: Zoned,
    data: Data,
    query: String,
    caret: usize,
    filter: Filter,
    filter_error: Option<String>,
    matches: Vec<usize>,
    cursor: usize,
    selected: Vec<String>,
    mode: Mode,
    menu_cursor: usize,
    targets: Vec<String>,
    status: Option<Status>,
    outcome: Option<Outcome>,
    loading_completed: bool,
}

impl State {
    pub(crate) fn new(data: Data, query: &str, now: Zoned) -> (Self, Option<Effect>) {
        let mut state = Self {
            now,
            data,
            query: query.to_owned(),
            caret: query.chars().count(),
            filter: Filter::parse("").expect("an empty expression has no tokens to reject"),
            filter_error: None,
            matches: Vec::new(),
            cursor: 0,
            selected: Vec::new(),
            mode: Mode::Filter,
            menu_cursor: 0,
            targets: Vec::new(),
            status: None,
            outcome: None,
            loading_completed: false,
        };
        let effect = state.refilter();
        (state, effect)
    }

    pub(crate) fn includes_completed(&self) -> bool {
        self.data.includes_completed
    }

    pub(crate) fn now(&self) -> &Zoned {
        &self.now
    }

    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    pub(crate) fn caret(&self) -> usize {
        self.caret
    }

    pub(crate) fn filter_error(&self) -> Option<&str> {
        self.filter_error.as_deref()
    }

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    pub(crate) fn menu_cursor(&self) -> usize {
        self.menu_cursor
    }

    pub(crate) fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub(crate) fn outcome(&self) -> Option<&Outcome> {
        self.outcome.as_ref()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub(crate) fn selected(&self) -> &[String] {
        &self.selected
    }

    pub(crate) fn is_selected(&self, id: &str) -> bool {
        self.selected.iter().any(|selected| selected == id)
    }

    pub(crate) fn lists(&self) -> &[ListChoice] {
        &self.data.lists
    }

    pub(crate) fn item(&self, id: &str) -> Option<&Item> {
        self.data.items.iter().find(|item| item.id == id)
    }

    pub(crate) fn targets(&self) -> &[String] {
        &self.targets
    }

    pub(crate) fn rows(&self) -> Vec<Row<'_>> {
        let mut rows: Vec<Row<'_>> = self
            .matches
            .iter()
            .map(|index| Row::Item(&self.data.items[*index]))
            .collect();
        if let Some(text) = self.add_text() {
            rows.push(Row::Add(text));
        }
        rows
    }

    pub(crate) fn current(&self) -> Option<Row<'_>> {
        if let Some(index) = self.matches.get(self.cursor) {
            Some(Row::Item(&self.data.items[*index]))
        } else if self.cursor == self.matches.len() {
            self.add_text().map(Row::Add)
        } else {
            None
        }
    }

    fn add_text(&self) -> Option<&str> {
        let text = self.query.trim();
        (!text.is_empty()).then_some(text)
    }

    fn row_count(&self) -> usize {
        self.matches.len() + usize::from(self.add_text().is_some())
    }

    fn current_id(&self) -> Option<String> {
        self.matches
            .get(self.cursor)
            .map(|index| self.data.items[*index].id.clone())
    }

    fn refilter(&mut self) -> Option<Effect> {
        match Filter::parse(&self.query) {
            Ok(filter) => {
                self.filter = filter;
                self.filter_error = None;
            }
            Err(error) => self.filter_error = Some(error.to_string()),
        }
        let now = &self.now;
        let filter = &self.filter;
        self.matches = self
            .data
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                filter.matches(
                    &FilterRow {
                        kind: FilterKind::Reminder,
                        title: &item.title,
                        collection: item.list.as_deref().unwrap_or_default(),
                        due: item.due,
                        due_time: item.due_time,
                        due_at: item.due_at,
                        priority: item.priority,
                        completed: item.completed,
                    },
                    now,
                )
            })
            .map(|(index, _)| index)
            .collect();
        self.cursor = self.cursor.min(self.row_count().saturating_sub(1));
        if self.filter.includes_completed()
            && !self.data.includes_completed
            && !self.loading_completed
        {
            self.loading_completed = true;
            Some(Effect::LoadCompleted)
        } else {
            None
        }
    }

    pub(crate) fn update(&mut self, action: Action) -> Option<Effect> {
        if action == Action::Quit {
            self.outcome = Some(Outcome::Cancelled);
            return None;
        }
        match self.mode {
            Mode::Filter => self.update_filter(action),
            Mode::Act => self.update_act(action),
            Mode::Priority => self.update_priority(action),
            Mode::Move => self.update_move(action),
            Mode::ConfirmDelete => self.update_confirm(action),
        }
    }

    fn update_filter(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Insert(character) => {
                let at = self.byte_offset(self.caret);
                self.query.insert(at, character);
                self.caret += 1;
                return self.edited();
            }
            Action::Backspace if self.caret > 0 => {
                self.caret -= 1;
                let at = self.byte_offset(self.caret);
                self.query.remove(at);
                return self.edited();
            }
            Action::DeleteForward if self.caret < self.query.chars().count() => {
                let at = self.byte_offset(self.caret);
                self.query.remove(at);
                return self.edited();
            }
            Action::CaretLeft => self.caret = self.caret.saturating_sub(1),
            Action::CaretRight => self.caret = (self.caret + 1).min(self.query.chars().count()),
            Action::CaretStart => self.caret = 0,
            Action::CaretEnd => self.caret = self.query.chars().count(),
            Action::KillToStart if self.caret > 0 => {
                let at = self.byte_offset(self.caret);
                self.query.replace_range(..at, "");
                self.caret = 0;
                return self.edited();
            }
            Action::KillToEnd if self.caret < self.query.chars().count() => {
                let at = self.byte_offset(self.caret);
                self.query.truncate(at);
                return self.edited();
            }
            Action::KillWord if self.caret > 0 => {
                let characters: Vec<char> = self.query.chars().collect();
                let mut start = self.caret;
                while start > 0 && characters[start - 1].is_whitespace() {
                    start -= 1;
                }
                while start > 0 && !characters[start - 1].is_whitespace() {
                    start -= 1;
                }
                let from = self.byte_offset(start);
                let to = self.byte_offset(self.caret);
                self.query.replace_range(from..to, "");
                self.caret = start;
                return self.edited();
            }
            Action::Up => self.move_cursor(false),
            Action::Down => self.move_cursor(true),
            Action::ToggleDown | Action::ToggleUp => {
                if let Some(id) = self.current_id() {
                    if let Some(position) = self.selected.iter().position(|item| *item == id) {
                        self.selected.remove(position);
                    } else {
                        self.selected.push(id);
                    }
                }
                self.move_cursor(action == Action::ToggleDown);
            }
            Action::Accept => return self.accept(),
            Action::OpenMenu => {
                self.targets = if self.selected.is_empty() {
                    self.current_id().into_iter().collect()
                } else {
                    self.selected.clone()
                };
                if self.targets.is_empty() {
                    self.set_status("Nothing to act on.", true);
                } else {
                    self.status = None;
                    self.open(Mode::Act);
                }
            }
            Action::Cancel => self.outcome = Some(Outcome::Cancelled),
            _ => {}
        }
        None
    }

    fn accept(&mut self) -> Option<Effect> {
        if !self.selected.is_empty() {
            self.outcome = Some(Outcome::Picked(self.selected.clone()));
            return None;
        }
        match self.current() {
            Some(Row::Item(item)) => {
                self.outcome = Some(Outcome::Picked(vec![item.id.clone()]));
                None
            }
            Some(Row::Add(text)) => Some(Effect::Add(text.to_owned())),
            None => None,
        }
    }

    fn update_act(&mut self, action: Action) -> Option<Effect> {
        let choice = match self.menu_action(action, ACTS.len()) {
            MenuStep::Chosen(index) => index,
            MenuStep::Hotkey(key) => ACTS.iter().position(|(hotkey, _)| *hotkey == key)?,
            MenuStep::Stay => return None,
        };
        match ACTS[choice].1 {
            "done" => {
                self.mode = Mode::Filter;
                Some(Effect::Change {
                    change: TaskChange::Complete,
                    ids: self.targets.clone(),
                })
            }
            "priority" => {
                self.open(Mode::Priority);
                None
            }
            "move" => {
                if self.data.lists.is_empty() {
                    self.mode = Mode::Filter;
                    self.set_status("No writable reminder lists.", true);
                } else {
                    self.open(Mode::Move);
                }
                None
            }
            _ => {
                self.open(Mode::ConfirmDelete);
                None
            }
        }
    }

    fn update_priority(&mut self, action: Action) -> Option<Effect> {
        let choice = match self.menu_action(action, PRIORITIES.len()) {
            MenuStep::Chosen(index) => index,
            MenuStep::Hotkey(key) => PRIORITIES.iter().position(|(hotkey, ..)| *hotkey == key)?,
            MenuStep::Stay => return None,
        };
        self.mode = Mode::Filter;
        Some(Effect::Change {
            change: TaskChange::Priority(PRIORITIES[choice].2),
            ids: self.targets.clone(),
        })
    }

    fn update_move(&mut self, action: Action) -> Option<Effect> {
        let choice = match self.menu_action(action, self.data.lists.len()) {
            MenuStep::Chosen(index) => index,
            MenuStep::Hotkey(key) => key
                .to_digit(10)
                .and_then(|digit| usize::try_from(digit).ok())
                .filter(|digit| (1..=self.data.lists.len()).contains(digit))?
                .saturating_sub(1),
            MenuStep::Stay => return None,
        };
        self.mode = Mode::Filter;
        Some(Effect::Change {
            change: TaskChange::Move(self.data.lists[choice].id.clone()),
            ids: self.targets.clone(),
        })
    }

    fn update_confirm(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::Hotkey('y') => {
                self.mode = Mode::Filter;
                Some(Effect::Delete(self.targets.clone()))
            }
            Action::Hotkey(_) | Action::Cancel => {
                self.mode = Mode::Filter;
                None
            }
            _ => None,
        }
    }

    fn menu_action(&mut self, action: Action, len: usize) -> MenuStep {
        match action {
            Action::Up | Action::ToggleUp => {
                self.menu_cursor = self.menu_cursor.checked_sub(1).unwrap_or(len - 1);
                MenuStep::Stay
            }
            Action::Down | Action::ToggleDown => {
                self.menu_cursor = (self.menu_cursor + 1) % len;
                MenuStep::Stay
            }
            Action::Accept => MenuStep::Chosen(self.menu_cursor),
            Action::Hotkey(key) => MenuStep::Hotkey(key),
            Action::Cancel => {
                self.mode = Mode::Filter;
                MenuStep::Stay
            }
            _ => MenuStep::Stay,
        }
    }

    fn open(&mut self, mode: Mode) {
        self.mode = mode;
        self.menu_cursor = 0;
    }

    pub(crate) fn resolve(&mut self, response: Response) -> Option<Effect> {
        match response {
            Response::Loaded(data) => {
                self.loading_completed = false;
                self.data = data;
                self.refilter()
            }
            Response::Added(item) => {
                let id = item.id.clone();
                self.data.items.push(item);
                self.outcome = Some(Outcome::Picked(vec![id]));
                None
            }
            Response::Changed { message, data } => {
                self.data = data;
                self.selected.clear();
                self.targets.clear();
                self.mode = Mode::Filter;
                self.set_status(&message, false);
                self.refilter()
            }
            Response::Failed(message) => {
                self.loading_completed = false;
                self.mode = Mode::Filter;
                self.set_status(&message, true);
                None
            }
        }
    }

    fn edited(&mut self) -> Option<Effect> {
        self.cursor = 0;
        self.status = None;
        self.refilter()
    }

    fn move_cursor(&mut self, down: bool) {
        let last = self.row_count().saturating_sub(1);
        self.cursor = if down {
            (self.cursor + 1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
    }

    fn byte_offset(&self, characters: usize) -> usize {
        self.query
            .char_indices()
            .nth(characters)
            .map_or(self.query.len(), |(offset, _)| offset)
    }

    fn set_status(&mut self, text: &str, error: bool) {
        self.status = Some(Status {
            text: text.to_owned(),
            error,
        });
    }
}

enum MenuStep {
    Chosen(usize),
    Hotkey(char),
    Stay,
}

#[cfg(test)]
pub(crate) mod tests {
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;

    pub(crate) fn now() -> Zoned {
        date(2026, 10, 30)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap()
    }

    pub(crate) fn item(
        id: &str,
        title: &str,
        list: &str,
        due: Option<Date>,
        priority: Option<Priority>,
    ) -> Item {
        Item {
            id: id.into(),
            title: title.into(),
            list: Some(list.into()),
            notes: None,
            url: None,
            due,
            due_time: None,
            due_at: None,
            priority,
            completed: false,
        }
    }

    pub(crate) fn data() -> Data {
        Data {
            items: vec![
                item(
                    "rem-1",
                    "Buy oat milk",
                    "Home",
                    Some(date(2026, 10, 30)),
                    Some(Priority::High),
                ),
                item("rem-2", "Buy eggs", "Work", None, None),
                item(
                    "rem-3",
                    "File report",
                    "Work",
                    Some(date(2026, 10, 31)),
                    Some(Priority::Medium),
                ),
            ],
            lists: vec![
                ListChoice {
                    id: "list-home".into(),
                    name: "Home".into(),
                },
                ListChoice {
                    id: "list-work".into(),
                    name: "Work".into(),
                },
            ],
            includes_completed: false,
        }
    }

    fn state(query: &str) -> State {
        let (state, effect) = State::new(data(), query, now());
        assert_eq!(effect, None);
        state
    }

    fn rows(state: &State) -> Vec<String> {
        state
            .rows()
            .iter()
            .map(|row| match row {
                Row::Item(item) => item.id.clone(),
                Row::Add(text) => format!("add:{text}"),
            })
            .collect()
    }

    pub(crate) fn type_text(state: &mut State, text: &str) -> Option<Effect> {
        let mut effect = None;
        for character in text.chars() {
            effect = state.update(Action::Insert(character)).or(effect);
        }
        effect
    }

    #[test]
    fn an_empty_query_lists_every_open_reminder_without_an_add_row() {
        let state = state("");
        assert_eq!(rows(&state), ["rem-1", "rem-2", "rem-3"]);
        assert_eq!(state.cursor(), 0);
        assert_eq!(state.mode(), Mode::Filter);
        assert_eq!(state.outcome(), None);
    }

    #[test]
    fn typing_filters_fuzzily_and_offers_an_add_row_last() {
        let mut state = state("");
        type_text(&mut state, "bym");
        assert_eq!(rows(&state), ["rem-1", "add:bym"]);
        type_text(&mut state, "zz");
        assert_eq!(rows(&state), ["add:bymzz"]);
        assert_eq!(state.match_count(), 0);
    }

    #[test]
    fn field_tokens_use_the_shared_filter_engine() {
        assert_eq!(rows(&state("pri:high")), ["rem-1", "add:pri:high"]);
        assert_eq!(
            rows(&state("list:work !due:none")),
            ["rem-3", "add:list:work !due:none"]
        );
        assert_eq!(rows(&state("due:today")), ["rem-1", "add:due:today"]);
    }

    #[test]
    fn parse_errors_show_inline_and_keep_the_last_good_results() {
        let start = state("pri:urgent");
        assert_eq!(
            start.filter_error(),
            Some("invalid value \"urgent\" for pri filter; expected high, medium, low, or none")
        );
        assert_eq!(rows(&start), ["rem-1", "rem-2", "rem-3", "add:pri:urgent"]);

        let mut state = state("eggs");
        type_text(&mut state, " !");
        assert_eq!(
            state.filter_error(),
            Some("filter token can't be empty after `!`")
        );
        assert_eq!(rows(&state), ["rem-2", "add:eggs !"]);
        state.update(Action::Backspace);
        assert_eq!(state.filter_error(), None);
    }

    #[test]
    fn readline_editing_moves_the_caret_and_kills_text() {
        let mut state = state("");
        type_text(&mut state, "buy oat");
        state.update(Action::KillWord);
        assert_eq!((state.query(), state.caret()), ("buy ", 4));
        state.update(Action::CaretStart);
        type_text(&mut state, "é");
        assert_eq!((state.query(), state.caret()), ("ébuy ", 1));
        state.update(Action::DeleteForward);
        assert_eq!(state.query(), "éuy ");
        state.update(Action::CaretRight);
        state.update(Action::KillToEnd);
        assert_eq!(state.query(), "éu");
        state.update(Action::CaretLeft);
        state.update(Action::Backspace);
        assert_eq!((state.query(), state.caret()), ("u", 0));
        state.update(Action::Backspace);
        assert_eq!(state.query(), "u");
        state.update(Action::CaretEnd);
        state.update(Action::KillToStart);
        assert_eq!((state.query(), state.caret()), ("", 0));
    }

    #[test]
    fn the_cursor_stays_within_the_rows_and_resets_when_the_query_changes() {
        let mut state = state("buy");
        state.update(Action::Up);
        assert_eq!(state.cursor(), 0);
        for _ in 0..5 {
            state.update(Action::Down);
        }
        assert_eq!(state.cursor(), 2);
        assert_eq!(state.current(), Some(Row::Add("buy")));
        type_text(&mut state, " eggs");
        assert_eq!(state.cursor(), 0);
    }

    #[test]
    fn tab_toggles_selection_and_enter_prints_every_selected_id() {
        let mut state = state("");
        state.update(Action::ToggleDown);
        assert_eq!(state.selected(), ["rem-1"]);
        assert_eq!(state.cursor(), 1);
        state.update(Action::Down);
        state.update(Action::ToggleUp);
        assert_eq!(state.selected(), ["rem-1", "rem-3"]);
        assert_eq!(state.cursor(), 1);
        state.update(Action::Up);
        state.update(Action::ToggleDown);
        assert_eq!(state.selected(), ["rem-3"]);
        state.update(Action::CaretStart);
        state.update(Action::ToggleDown);
        type_text(&mut state, "eggs");
        assert_eq!(rows(&state), ["rem-2", "add:eggs"]);
        state.update(Action::Accept);
        assert_eq!(
            state.outcome(),
            Some(&Outcome::Picked(vec!["rem-3".into(), "rem-2".into()]))
        );
    }

    #[test]
    fn enter_picks_the_highlighted_reminder() {
        let mut state = state("report");
        assert_eq!(state.update(Action::Accept), None);
        assert_eq!(
            state.outcome(),
            Some(&Outcome::Picked(vec!["rem-3".into()]))
        );
    }

    #[test]
    fn tab_on_the_add_row_selects_nothing() {
        let mut state = state("zzz");
        state.update(Action::ToggleDown);
        assert_eq!(state.selected(), Vec::<String>::new());
    }

    #[test]
    fn enter_on_the_add_row_adds_the_trimmed_query_and_picks_the_new_id() {
        let mut state = state("");
        type_text(&mut state, "  Call mom fri !high ");
        assert_eq!(
            state.update(Action::Accept),
            Some(Effect::Add("Call mom fri !high".into()))
        );
        assert_eq!(state.outcome(), None);
        let added = item("rem-9", "Call mom", "Home", None, Some(Priority::High));
        assert_eq!(state.resolve(Response::Added(added.clone())), None);
        assert_eq!(
            state.outcome(),
            Some(&Outcome::Picked(vec!["rem-9".into()]))
        );
        assert_eq!(state.item("rem-9"), Some(&added));
    }

    #[test]
    fn a_failed_add_reports_the_error_and_keeps_the_prompt_open() {
        let mut state = state("Pay !urgent");
        state.update(Action::Down);
        assert_eq!(
            state.update(Action::Accept),
            Some(Effect::Add("Pay !urgent".into()))
        );
        state.resolve(Response::Failed("invalid inline priority".into()));
        assert_eq!(state.outcome(), None);
        assert_eq!(
            state.status(),
            Some(&Status {
                text: "invalid inline priority".into(),
                error: true
            })
        );
    }

    #[test]
    fn enter_with_no_rows_does_nothing() {
        let (mut state, _) = State::new(Data::default(), "", now());
        assert_eq!(state.update(Action::Accept), None);
        assert_eq!(state.outcome(), None);
    }

    #[test]
    fn escape_cancels_from_the_prompt_but_only_closes_menus() {
        let mut state = state("");
        state.update(Action::OpenMenu);
        state.update(Action::Cancel);
        assert_eq!((state.mode(), state.outcome()), (Mode::Filter, None));
        state.update(Action::Cancel);
        assert_eq!(state.outcome(), Some(&Outcome::Cancelled));

        let mut state = self::state("");
        state.update(Action::OpenMenu);
        state.update(Action::Quit);
        assert_eq!(state.outcome(), Some(&Outcome::Cancelled));
    }

    #[test]
    fn the_act_menu_needs_a_target() {
        let mut state = state("zzz");
        assert_eq!(state.update(Action::OpenMenu), None);
        assert_eq!(state.mode(), Mode::Filter);
        assert_eq!(state.status().map(|status| status.error), Some(true));
    }

    #[test]
    fn done_acts_on_the_highlighted_reminder_or_the_selection() {
        let mut state = state("");
        state.update(Action::Down);
        state.update(Action::OpenMenu);
        assert_eq!(
            (state.mode(), state.targets()),
            (Mode::Act, &["rem-2".to_owned()][..])
        );
        assert_eq!(
            state.update(Action::Hotkey('x')),
            Some(Effect::Change {
                change: TaskChange::Complete,
                ids: vec!["rem-2".into()]
            })
        );
        assert_eq!(state.mode(), Mode::Filter);

        state.update(Action::ToggleDown);
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        assert_eq!(
            state.update(Action::Accept),
            Some(Effect::Change {
                change: TaskChange::Complete,
                ids: vec!["rem-2".into(), "rem-3".into()]
            })
        );
    }

    #[test]
    fn priority_is_chosen_by_letter_or_by_moving_and_pressing_enter() {
        let mut state = state("");
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('p'));
        assert_eq!(state.mode(), Mode::Priority);
        assert_eq!(state.update(Action::Hotkey('q')), None);
        assert_eq!(
            state.update(Action::Hotkey('n')),
            Some(Effect::Change {
                change: TaskChange::Priority(None),
                ids: vec!["rem-1".into()]
            })
        );

        state.update(Action::OpenMenu);
        state.update(Action::Down);
        state.update(Action::Accept);
        assert_eq!(state.mode(), Mode::Priority);
        state.update(Action::Up);
        assert_eq!(state.menu_cursor(), 3);
        state.update(Action::Up);
        assert_eq!(
            state.update(Action::Accept),
            Some(Effect::Change {
                change: TaskChange::Priority(Some(Priority::Low)),
                ids: vec!["rem-1".into()]
            })
        );
    }

    #[test]
    fn move_targets_a_writable_list_by_number_or_cursor() {
        let mut state = state("");
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('m'));
        assert_eq!(state.mode(), Mode::Move);
        assert_eq!(state.update(Action::Hotkey('9')), None);
        assert_eq!(state.mode(), Mode::Move);
        assert_eq!(
            state.update(Action::Hotkey('2')),
            Some(Effect::Change {
                change: TaskChange::Move("list-work".into()),
                ids: vec!["rem-1".into()]
            })
        );

        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('m'));
        state.update(Action::Down);
        state.update(Action::Down);
        assert_eq!(
            state.update(Action::Accept),
            Some(Effect::Change {
                change: TaskChange::Move("list-home".into()),
                ids: vec!["rem-1".into()]
            })
        );
    }

    #[test]
    fn move_without_writable_lists_reports_an_error() {
        let mut empty = data();
        empty.lists.clear();
        let (mut state, _) = State::new(empty, "", now());
        state.update(Action::OpenMenu);
        assert_eq!(state.update(Action::Hotkey('m')), None);
        assert_eq!(state.mode(), Mode::Filter);
        assert_eq!(state.status().map(|status| status.error), Some(true));
    }

    #[test]
    fn delete_asks_for_confirmation() {
        let mut state = state("");
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        assert_eq!(state.mode(), Mode::ConfirmDelete);
        assert_eq!(state.update(Action::Accept), None);
        assert_eq!(state.mode(), Mode::ConfirmDelete);
        assert_eq!(state.update(Action::Hotkey('n')), None);
        assert_eq!(state.mode(), Mode::Filter);

        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('d'));
        assert_eq!(
            state.update(Action::Hotkey('y')),
            Some(Effect::Delete(vec!["rem-1".into()]))
        );
        assert_eq!(state.mode(), Mode::Filter);
    }

    #[test]
    fn a_finished_change_reloads_data_and_clears_the_selection() {
        let mut state = state("");
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('x'));
        let mut reloaded = data();
        reloaded.items.remove(0);
        assert_eq!(
            state.resolve(Response::Changed {
                message: "Completed \"Buy oat milk\"".into(),
                data: reloaded,
            }),
            None
        );
        assert_eq!(rows(&state), ["rem-2", "rem-3"]);
        assert_eq!(state.selected(), Vec::<String>::new());
        assert_eq!(
            state.status(),
            Some(&Status {
                text: "Completed \"Buy oat milk\"".into(),
                error: false
            })
        );
    }

    #[test]
    fn a_failed_change_keeps_the_selection() {
        let mut state = state("");
        state.update(Action::ToggleDown);
        state.update(Action::OpenMenu);
        state.update(Action::Hotkey('x'));
        state.resolve(Response::Failed(
            "\"Buy oat milk\" is already completed".into(),
        ));
        assert_eq!(state.selected(), ["rem-1"]);
        assert_eq!(state.mode(), Mode::Filter);
        assert_eq!(state.status().map(|status| status.error), Some(true));
    }

    #[test]
    fn is_done_loads_completed_reminders_once() {
        let mut state = state("");
        assert_eq!(
            type_text(&mut state, "is:done"),
            Some(Effect::LoadCompleted)
        );
        assert_eq!(rows(&state), ["add:is:done"]);
        assert_eq!(state.update(Action::Insert(' ')), None);
        let mut completed = data();
        completed.includes_completed = true;
        let mut done = item("rem-7", "Call the dentist", "Home", None, None);
        done.completed = true;
        completed.items.push(done);
        assert_eq!(state.resolve(Response::Loaded(completed)), None);
        assert_eq!(rows(&state), ["rem-7", "add:is:done"]);
        assert!(state.includes_completed());

        let (state, effect) = State::new(data(), "is:done", now());
        assert_eq!(effect, Some(Effect::LoadCompleted));
        assert_eq!(state.match_count(), 0);
    }
}

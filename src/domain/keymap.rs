use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

pub const DEFAULT_PRESET: &str = "default";

#[cfg(test)]
pub(crate) const TEST_PRESET: &str = "test-alternate";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Browse,
    Filter,
    Capture,
}

impl State {
    pub const ALL: [Self; 3] = [Self::Browse, Self::Filter, Self::Capture];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Filter => "filter",
            Self::Capture => "capture",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for State {
    type Err = StateParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "browse" => Ok(Self::Browse),
            "filter" => Ok(Self::Filter),
            "capture" => Ok(Self::Capture),
            _ => Err(StateParseError {
                value: value.to_owned(),
                suggestion: closest(value, State::ALL.map(State::as_str)),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateParseError {
    value: String,
    suggestion: Option<&'static str>,
}

impl fmt::Display for StateParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown keymap state {:?}; ", self.value)?;
        write_suggestion(formatter, self.suggestion, &State::ALL.map(State::as_str))
    }
}

impl std::error::Error for StateParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Filter,
    Capture,
    ClearFilter,
    KeepFilter,
    SelectToggle,
    SelectAll,
    ActMenu,
    Done,
    Reopen,
    PriorityHigh,
    PriorityMedium,
    PriorityLow,
    PriorityNone,
    Move,
    Edit,
    Delete,
    Add,
    Quit,
    Help,
    Accept,
    Cancel,
    NextPane,
    PrevPane,
    PreviewToggle,
    None,
}

impl Action {
    pub const ALL: [Self; 31] = [
        Self::Up,
        Self::Down,
        Self::PageUp,
        Self::PageDown,
        Self::Top,
        Self::Bottom,
        Self::Filter,
        Self::Capture,
        Self::ClearFilter,
        Self::KeepFilter,
        Self::SelectToggle,
        Self::SelectAll,
        Self::ActMenu,
        Self::Done,
        Self::Reopen,
        Self::PriorityHigh,
        Self::PriorityMedium,
        Self::PriorityLow,
        Self::PriorityNone,
        Self::Move,
        Self::Edit,
        Self::Delete,
        Self::Add,
        Self::Quit,
        Self::Help,
        Self::Accept,
        Self::Cancel,
        Self::NextPane,
        Self::PrevPane,
        Self::PreviewToggle,
        Self::None,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::PageUp => "page-up",
            Self::PageDown => "page-down",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Filter => "filter",
            Self::Capture => "capture",
            Self::ClearFilter => "clear-filter",
            Self::KeepFilter => "keep-filter",
            Self::SelectToggle => "select-toggle",
            Self::SelectAll => "select-all",
            Self::ActMenu => "act-menu",
            Self::Done => "done",
            Self::Reopen => "reopen",
            Self::PriorityHigh => "priority-high",
            Self::PriorityMedium => "priority-medium",
            Self::PriorityLow => "priority-low",
            Self::PriorityNone => "priority-none",
            Self::Move => "move",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Add => "add",
            Self::Quit => "quit",
            Self::Help => "help",
            Self::Accept => "accept",
            Self::Cancel => "cancel",
            Self::NextPane => "next-pane",
            Self::PrevPane => "prev-pane",
            Self::PreviewToggle => "preview-toggle",
            Self::None => "none",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Up => "move up",
            Self::Down => "move down",
            Self::PageUp => "move one page up",
            Self::PageDown => "move one page down",
            Self::Top => "move to the first item",
            Self::Bottom => "move to the last item",
            Self::Filter => "enter filter mode",
            Self::Capture => "enter capture mode",
            Self::ClearFilter => "clear the filter",
            Self::KeepFilter => "keep the current filter",
            Self::SelectToggle => "toggle the current selection",
            Self::SelectAll => "select all visible items",
            Self::ActMenu => "open the action menu",
            Self::Done => "complete selected reminders",
            Self::Reopen => "reopen selected reminders",
            Self::PriorityHigh => "set high priority",
            Self::PriorityMedium => "set medium priority",
            Self::PriorityLow => "set low priority",
            Self::PriorityNone => "clear priority",
            Self::Move => "move selected reminders",
            Self::Edit => "edit selected reminders",
            Self::Delete => "delete selected reminders",
            Self::Add => "add a reminder",
            Self::Quit => "quit",
            Self::Help => "show key help",
            Self::Accept => "accept",
            Self::Cancel => "cancel",
            Self::NextPane => "focus the next pane",
            Self::PrevPane => "focus the previous pane",
            Self::PreviewToggle => "toggle the preview",
            Self::None => "remove this binding",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Action {
    type Err = ActionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Action::ALL
            .into_iter()
            .find(|action| action.as_str() == value)
            .ok_or_else(|| ActionParseError {
                value: value.to_owned(),
                suggestion: closest(value, Action::ALL.map(Action::as_str)),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionParseError {
    value: String,
    suggestion: Option<&'static str>,
}

impl fmt::Display for ActionParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown keymap action {:?}; ", self.value)?;
        write_suggestion(formatter, self.suggestion, &Action::ALL.map(Action::as_str))
    }
}

impl std::error::Error for ActionParseError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyChord {
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Key {
    Character(char),
    Enter,
    Escape,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Insert,
    Function(u8),
    Space,
}

impl fmt::Display for KeyChord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            formatter.write_str("ctrl-")?;
        }
        if self.alt {
            formatter.write_str("alt-")?;
        }
        if self.shift {
            formatter.write_str("shift-")?;
        }
        match self.key {
            Key::Character(value) => write!(formatter, "{value}"),
            Key::Enter => formatter.write_str("enter"),
            Key::Escape => formatter.write_str("esc"),
            Key::Tab => formatter.write_str("tab"),
            Key::Up => formatter.write_str("up"),
            Key::Down => formatter.write_str("down"),
            Key::Left => formatter.write_str("left"),
            Key::Right => formatter.write_str("right"),
            Key::Home => formatter.write_str("home"),
            Key::End => formatter.write_str("end"),
            Key::PageUp => formatter.write_str("page-up"),
            Key::PageDown => formatter.write_str("page-down"),
            Key::Backspace => formatter.write_str("backspace"),
            Key::Delete => formatter.write_str("delete"),
            Key::Insert => formatter.write_str("insert"),
            Key::Function(number) => write!(formatter, "f{number}"),
            Key::Space => formatter.write_str("space"),
        }
    }
}

impl FromStr for KeyChord {
    type Err = KeyChordParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err(KeyChordParseError::new(
                value,
                "a key chord cannot be empty",
            ));
        }
        if value.trim() != value {
            return Err(KeyChordParseError::new(
                value,
                "a key chord cannot start or end with whitespace; use `space` for the space key",
            ));
        }
        let mut rest = value;
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        while let Some((prefix, tail)) = rest.split_once('-') {
            let modifier = prefix.to_ascii_lowercase();
            let target = match modifier.as_str() {
                "ctrl" => &mut ctrl,
                "alt" => &mut alt,
                "shift" => &mut shift,
                _ => break,
            };
            if *target {
                return Err(KeyChordParseError::new(
                    value,
                    format!("modifier `{modifier}` appears more than once"),
                ));
            }
            *target = true;
            rest = tail;
        }
        if rest.is_empty() {
            return Err(KeyChordParseError::new(
                value,
                "a modifier must be followed by a key",
            ));
        }
        let lower = rest.to_ascii_lowercase();
        let key = match lower.as_str() {
            "enter" => Key::Enter,
            "esc" => Key::Escape,
            "tab" => Key::Tab,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "page-up" => Key::PageUp,
            "page-down" => Key::PageDown,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "insert" => Key::Insert,
            "space" => Key::Space,
            _ => parse_function_key(&lower).map_or_else(
                || parse_character(value, rest, ctrl || alt || shift, &mut shift),
                Ok,
            )?,
        };
        Ok(Self {
            ctrl,
            alt,
            shift,
            key,
        })
    }
}

fn parse_function_key(value: &str) -> Option<Key> {
    let digits = value.strip_prefix('f')?;
    if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = digits.parse::<u8>().ok()?;
    (1..=24).contains(&number).then_some(Key::Function(number))
}

fn parse_character(
    chord: &str,
    value: &str,
    has_modifier: bool,
    shift: &mut bool,
) -> Result<Key, KeyChordParseError> {
    let mut characters = value.chars();
    let Some(mut character) = characters.next() else {
        return Err(KeyChordParseError::new(chord, "a key is required"));
    };
    if characters.next().is_some() || character.is_control() || character.is_whitespace() {
        return Err(KeyChordParseError::new(
            chord,
            "expected one printable character or a named key such as `enter`, `esc`, `up`, `f1`, or `space`",
        ));
    }
    if character.is_ascii_uppercase() {
        character = character.to_ascii_lowercase();
        if !has_modifier {
            *shift = true;
        }
    }
    Ok(Key::Character(character))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChordParseError {
    value: String,
    reason: String,
}

impl KeyChordParseError {
    fn new(value: &str, reason: impl Into<String>) -> Self {
        Self {
            value: value.to_owned(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for KeyChordParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid key chord {:?}: {}",
            self.value, self.reason
        )
    }
}

impl std::error::Error for KeyChordParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    states: BTreeMap<State, BTreeMap<KeyChord, Vec<Action>>>,
}

impl Keymap {
    pub fn from_preset(name: &str) -> Result<Self, PresetError> {
        let mut keymap = Self {
            states: State::ALL
                .into_iter()
                .map(|state| (state, BTreeMap::new()))
                .collect(),
        };
        match name {
            DEFAULT_PRESET => add_default_preset(&mut keymap),
            #[cfg(test)]
            TEST_PRESET => {
                add_default_preset(&mut keymap);
                bind_default(&mut keymap, State::Browse, "x", &[Action::None]);
                bind_default(&mut keymap, State::Browse, "z", &[Action::Done]);
            }
            _ => {
                return Err(PresetError {
                    value: name.to_owned(),
                    suggestion: closest(name, [DEFAULT_PRESET]),
                });
            }
        }
        Ok(keymap)
    }

    pub fn actions(&self, state: State, chord: &KeyChord) -> Option<&[Action]> {
        self.states
            .get(&state)
            .and_then(|bindings| bindings.get(chord))
            .map(Vec::as_slice)
    }

    pub fn bindings(&self, state: State) -> impl Iterator<Item = (&KeyChord, &[Action])> {
        self.states
            .get(&state)
            .into_iter()
            .flat_map(|bindings| bindings.iter())
            .map(|(chord, actions)| (chord, actions.as_slice()))
    }

    pub(crate) fn bind(&mut self, state: State, chord: KeyChord, actions: Vec<Action>) {
        let bindings = self
            .states
            .get_mut(&state)
            .expect("every keymap contains all input states");
        if actions == [Action::None] {
            bindings.remove(&chord);
        } else {
            bindings.insert(chord, actions);
        }
    }

    pub(crate) fn has_action(&self, state: State, expected: impl Fn(Action) -> bool) -> bool {
        self.bindings(state)
            .any(|(_, actions)| actions.iter().copied().any(&expected))
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::from_preset(DEFAULT_PRESET).expect("the built-in keymap preset is valid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetError {
    value: String,
    suggestion: Option<&'static str>,
}

impl fmt::Display for PresetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown keymap preset {:?}; ", self.value)?;
        write_suggestion(formatter, self.suggestion, &[DEFAULT_PRESET])
    }
}

impl std::error::Error for PresetError {}

fn add_default_preset(keymap: &mut Keymap) {
    add_browse_defaults(keymap);
    add_filter_defaults(keymap);
    add_capture_defaults(keymap);
}

fn add_browse_defaults(keymap: &mut Keymap) {
    for chord in ["up", "k", "ctrl-p"] {
        bind_default(keymap, State::Browse, chord, &[Action::Up]);
    }
    for chord in ["down", "j", "ctrl-n"] {
        bind_default(keymap, State::Browse, chord, &[Action::Down]);
    }
    for chord in ["page-up", "ctrl-u"] {
        bind_default(keymap, State::Browse, chord, &[Action::PageUp]);
    }
    for chord in ["page-down", "ctrl-d"] {
        bind_default(keymap, State::Browse, chord, &[Action::PageDown]);
    }
    for chord in ["home", "g", "ctrl-a"] {
        bind_default(keymap, State::Browse, chord, &[Action::Top]);
    }
    for chord in ["end", "shift-g", "ctrl-e"] {
        bind_default(keymap, State::Browse, chord, &[Action::Bottom]);
    }
    bind_default(keymap, State::Browse, "/", &[Action::Filter]);
    bind_default(keymap, State::Browse, "a", &[Action::Capture]);
    bind_default(keymap, State::Browse, "ctrl-l", &[Action::ClearFilter]);
    bind_default(keymap, State::Browse, "alt-k", &[Action::KeepFilter]);
    for chord in ["space", "tab"] {
        bind_default(keymap, State::Browse, chord, &[Action::SelectToggle]);
    }
    bind_default(keymap, State::Browse, "alt-a", &[Action::SelectAll]);
    bind_default(keymap, State::Browse, "ctrl-o", &[Action::ActMenu]);
    bind_default(keymap, State::Browse, "x", &[Action::Done]);
    bind_default(keymap, State::Browse, "r", &[Action::Reopen]);
    bind_default(keymap, State::Browse, "1", &[Action::PriorityHigh]);
    bind_default(keymap, State::Browse, "2", &[Action::PriorityMedium]);
    bind_default(keymap, State::Browse, "3", &[Action::PriorityLow]);
    bind_default(keymap, State::Browse, "0", &[Action::PriorityNone]);
    bind_default(keymap, State::Browse, "m", &[Action::Move]);
    bind_default(keymap, State::Browse, "e", &[Action::Edit]);
    for chord in ["delete", "backspace"] {
        bind_default(keymap, State::Browse, chord, &[Action::Delete]);
    }
    bind_default(keymap, State::Browse, "n", &[Action::Add]);
    for chord in ["q", "esc", "ctrl-c"] {
        bind_default(keymap, State::Browse, chord, &[Action::Quit]);
    }
    for chord in ["?", "f1"] {
        bind_default(keymap, State::Browse, chord, &[Action::Help]);
    }
    bind_default(keymap, State::Browse, "enter", &[Action::Accept]);
    for chord in ["right", "]"] {
        bind_default(keymap, State::Browse, chord, &[Action::NextPane]);
    }
    bind_default(
        keymap,
        State::Browse,
        "shift-tab",
        &[Action::SelectToggle, Action::Up],
    );
    for chord in ["left", "["] {
        bind_default(keymap, State::Browse, chord, &[Action::PrevPane]);
    }
    bind_default(keymap, State::Browse, "p", &[Action::PreviewToggle]);
}

fn add_filter_defaults(keymap: &mut Keymap) {
    for chord in ["up", "ctrl-p"] {
        bind_default(keymap, State::Filter, chord, &[Action::Up]);
    }
    for chord in ["down", "ctrl-n"] {
        bind_default(keymap, State::Filter, chord, &[Action::Down]);
    }
    bind_default(keymap, State::Filter, "page-up", &[Action::PageUp]);
    bind_default(keymap, State::Filter, "page-down", &[Action::PageDown]);
    for chord in ["home", "ctrl-a"] {
        bind_default(keymap, State::Filter, chord, &[Action::Top]);
    }
    for chord in ["end", "ctrl-e"] {
        bind_default(keymap, State::Filter, chord, &[Action::Bottom]);
    }
    bind_default(keymap, State::Filter, "ctrl-u", &[Action::ClearFilter]);
    bind_default(keymap, State::Filter, "alt-enter", &[Action::KeepFilter]);
    bind_default(keymap, State::Filter, "tab", &[Action::SelectToggle]);
    bind_default(
        keymap,
        State::Filter,
        "shift-tab",
        &[Action::SelectToggle, Action::Up],
    );
    bind_default(keymap, State::Filter, "alt-a", &[Action::SelectAll]);
    bind_default(keymap, State::Filter, "ctrl-o", &[Action::ActMenu]);
    bind_default(keymap, State::Filter, "insert", &[Action::Add]);
    bind_default(keymap, State::Filter, "enter", &[Action::Accept]);
    for chord in ["esc", "ctrl-c"] {
        bind_default(keymap, State::Filter, chord, &[Action::Cancel]);
    }
    bind_default(keymap, State::Filter, "ctrl-q", &[Action::Quit]);
    bind_default(keymap, State::Filter, "f1", &[Action::Help]);
    bind_default(keymap, State::Filter, "right", &[Action::NextPane]);
    bind_default(keymap, State::Filter, "left", &[Action::PrevPane]);
    bind_default(
        keymap,
        State::Filter,
        "ctrl-space",
        &[Action::PreviewToggle],
    );
}

fn add_capture_defaults(keymap: &mut Keymap) {
    for chord in ["up", "ctrl-p"] {
        bind_default(keymap, State::Capture, chord, &[Action::Up]);
    }
    for chord in ["down", "ctrl-n"] {
        bind_default(keymap, State::Capture, chord, &[Action::Down]);
    }
    bind_default(keymap, State::Capture, "enter", &[Action::Accept]);
    for chord in ["esc", "ctrl-c"] {
        bind_default(keymap, State::Capture, chord, &[Action::Cancel]);
    }
    bind_default(keymap, State::Capture, "ctrl-q", &[Action::Quit]);
    bind_default(keymap, State::Capture, "f1", &[Action::Help]);
    bind_default(keymap, State::Capture, "alt-h", &[Action::PriorityHigh]);
    bind_default(keymap, State::Capture, "alt-m", &[Action::PriorityMedium]);
    bind_default(keymap, State::Capture, "alt-l", &[Action::PriorityLow]);
    bind_default(keymap, State::Capture, "alt-n", &[Action::PriorityNone]);
}

fn bind_default(keymap: &mut Keymap, state: State, chord: &str, actions: &[Action]) {
    keymap.bind(
        state,
        chord
            .parse()
            .expect("built-in key chords use the supported canonical syntax"),
        actions.to_vec(),
    );
}

fn closest<const N: usize>(value: &str, candidates: [&'static str; N]) -> Option<&'static str> {
    let limit = (value.chars().count() / 3).max(2);
    candidates
        .into_iter()
        .map(|candidate| (edit_distance(value, candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn write_suggestion(
    formatter: &mut fmt::Formatter<'_>,
    suggestion: Option<&str>,
    candidates: &[&str],
) -> fmt::Result {
    if let Some(suggestion) = suggestion {
        return write!(formatter, "did you mean {suggestion:?}?");
    }
    formatter.write_str("expected ")?;
    match candidates {
        [only] => formatter.write_str(only),
        [first, second] => write!(formatter, "{first} or {second}"),
        [rest @ .., last] => write!(formatter, "{}, or {last}", rest.join(", ")),
        [] => Ok(()),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution = usize::from(left_character != *right_character);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(previous[right_index] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_chords_have_a_case_insensitive_canonical_form() {
        let cases = [
            ("x", "x"),
            ("G", "shift-g"),
            ("Ctrl-O", "ctrl-o"),
            ("ALT-X", "alt-x"),
            ("enter", "enter"),
            ("ESC", "esc"),
            ("shift-tab", "shift-tab"),
            ("page-up", "page-up"),
            ("F12", "f12"),
            ("space", "space"),
        ];
        for (input, canonical) in cases {
            assert_eq!(input.parse::<KeyChord>().unwrap().to_string(), canonical);
        }
        assert_eq!(
            "ctrl-o".parse::<KeyChord>().unwrap(),
            "Ctrl-O".parse::<KeyChord>().unwrap()
        );
    }

    #[test]
    fn invalid_chords_and_actions_are_helpful() {
        assert_eq!(
            "ctrl-".parse::<KeyChord>().unwrap_err().to_string(),
            "invalid key chord \"ctrl-\": a modifier must be followed by a key"
        );
        assert_eq!(
            "dne".parse::<Action>().unwrap_err().to_string(),
            "unknown keymap action \"dne\"; did you mean \"done\"?"
        );
        assert_eq!(
            "brows".parse::<State>().unwrap_err().to_string(),
            "unknown keymap state \"brows\"; did you mean \"browse\"?"
        );
    }

    #[test]
    fn none_removes_a_preset_binding() {
        let chord = "x".parse::<KeyChord>().unwrap();
        let mut keymap = Keymap::default();
        assert_eq!(
            keymap.actions(State::Browse, &chord),
            Some(&[Action::Done][..])
        );
        keymap.bind(State::Browse, chord.clone(), vec![Action::None]);
        assert_eq!(keymap.actions(State::Browse, &chord), None);
    }

    #[test]
    fn unknown_preset_has_a_suggestion() {
        assert_eq!(
            Keymap::from_preset("defaults").unwrap_err().to_string(),
            "unknown keymap preset \"defaults\"; did you mean \"default\"?"
        );
    }

    #[test]
    fn suggestions_are_offered_only_for_close_names() {
        assert_eq!(
            "zzzzzz".parse::<State>().unwrap_err().to_string(),
            "unknown keymap state \"zzzzzz\"; expected browse, filter, or capture"
        );
        assert_eq!(
            Keymap::from_preset("vim").unwrap_err().to_string(),
            "unknown keymap preset \"vim\"; expected default"
        );
        let error = "frobnicate".parse::<Action>().unwrap_err().to_string();
        assert!(
            error.starts_with("unknown keymap action \"frobnicate\"; expected up, down, "),
            "{error}"
        );
        assert!(error.ends_with(", preview-toggle, or none"), "{error}");
    }

    #[test]
    fn function_keys_accept_only_canonical_numbers() {
        for valid in ["f1", "F12", "f24"] {
            assert!(valid.parse::<KeyChord>().is_ok(), "{valid}");
        }
        for invalid in ["f0", "f25", "f+1", "f01", "f001"] {
            assert!(invalid.parse::<KeyChord>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn hyphen_and_shifted_letters_parse_as_distinct_chords() {
        let parse = |chord: &str| chord.parse::<KeyChord>().unwrap();
        assert_eq!(parse("-").to_string(), "-");
        assert_eq!(parse("ctrl--").to_string(), "ctrl--");
        assert_eq!(parse("alt-shift--").to_string(), "alt-shift--");
        assert_eq!(parse("shift-G"), parse("G"));
        assert_eq!(parse("ctrl-G"), parse("ctrl-g"));
        assert_ne!(parse("ctrl-G"), parse("ctrl-shift-g"));
        assert_eq!(parse("ctrl-shift-g").to_string(), "ctrl-shift-g");
        assert!("ctrl-ctrl-x".parse::<KeyChord>().is_err());
        assert!(" x".parse::<KeyChord>().is_err());
        assert!("".parse::<KeyChord>().is_err());
    }
}

use jiff::ToSpan;
use jiff::civil::Date;
use miette::Diagnostic;
use thiserror::Error;

use crate::domain::Priority;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Reminder,
    Event,
}

#[derive(Debug, Clone, Copy)]
pub struct FilterRow<'a> {
    pub kind: FilterKind,
    pub title: &'a str,
    pub collection: &'a str,
    pub due: Option<Date>,
    pub priority: Option<Priority>,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    clauses: Vec<Clause>,
    include_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Clause {
    predicate: Predicate,
    negated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Predicate {
    Text(String),
    List(String),
    Calendar(String),
    Due(Due),
    Priority(Option<Priority>),
    Is(Is),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Due {
    Today,
    Overdue,
    Week,
    None,
    Exact(Date),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Is {
    Overdue,
    Done,
    Open,
    Event,
}

#[derive(Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum FilterError {
    #[error("invalid value {value:?} for {field} filter; expected {expected}")]
    #[diagnostic(code(nib::filter::invalid_value))]
    InvalidValue {
        field: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("filter token can't be empty after `!`")]
    #[diagnostic(code(nib::filter::empty_token))]
    EmptyToken,
}

impl Filter {
    pub fn parse(expression: &str) -> Result<Self, FilterError> {
        let mut clauses = Vec::new();
        let mut include_completed = false;
        for token in expression.split_whitespace() {
            let (negated, token) = token
                .strip_prefix('!')
                .map_or((false, token), |token| (true, token));
            if token.is_empty() {
                return Err(FilterError::EmptyToken);
            }
            let predicate = parse_predicate(token)?;
            if !negated && predicate == Predicate::Is(Is::Done) {
                include_completed = true;
            }
            clauses.push(Clause { predicate, negated });
        }
        Ok(Self {
            clauses,
            include_completed,
        })
    }

    pub fn matches(&self, row: &FilterRow<'_>, today: Date) -> bool {
        if row.completed && !self.include_completed {
            return false;
        }
        self.clauses
            .iter()
            .all(|clause| clause.predicate.matches(row, today) != clause.negated)
    }

    pub(crate) fn includes_completed(&self) -> bool {
        self.include_completed
    }
}

impl Predicate {
    fn matches(&self, row: &FilterRow<'_>, today: Date) -> bool {
        match self {
            Self::Text(value) => {
                fuzzy_contains(row.title, value) || fuzzy_contains(row.collection, value)
            }
            Self::List(value) => {
                row.kind == FilterKind::Reminder && contains_case_insensitive(row.collection, value)
            }
            Self::Calendar(value) => {
                row.kind == FilterKind::Event && contains_case_insensitive(row.collection, value)
            }
            Self::Due(value) => value.matches(row.due, today),
            Self::Priority(value) => row.priority == *value,
            Self::Is(value) => value.matches(row, today),
        }
    }
}

impl Due {
    fn matches(self, due: Option<Date>, today: Date) -> bool {
        match self {
            Self::Today => due == Some(today),
            Self::Overdue => due.is_some_and(|due| due < today),
            Self::Week => due.is_some_and(|due| {
                due < today || today.checked_add(7.days()).map_or(true, |end| due <= end)
            }),
            Self::None => due.is_none(),
            Self::Exact(date) => due == Some(date),
        }
    }
}

impl Is {
    fn matches(self, row: &FilterRow<'_>, today: Date) -> bool {
        match self {
            Self::Overdue => row.due.is_some_and(|due| due < today),
            Self::Done => row.completed,
            Self::Open => !row.completed,
            Self::Event => row.kind == FilterKind::Event,
        }
    }
}

fn parse_predicate(token: &str) -> Result<Predicate, FilterError> {
    let Some((field, value)) = token.split_once(':') else {
        return Ok(Predicate::Text(token.to_lowercase()));
    };
    match field.to_ascii_lowercase().as_str() {
        "list" => non_empty_value("list", value).map(|value| Predicate::List(value.to_lowercase())),
        "cal" => {
            non_empty_value("cal", value).map(|value| Predicate::Calendar(value.to_lowercase()))
        }
        "due" => parse_due(value).map(Predicate::Due),
        "pri" => parse_priority(value).map(Predicate::Priority),
        "is" => parse_is(value).map(Predicate::Is),
        _ => Ok(Predicate::Text(token.to_lowercase())),
    }
}

fn non_empty_value<'a>(field: &'static str, value: &'a str) -> Result<&'a str, FilterError> {
    if value.is_empty() {
        Err(invalid_value(field, value, "a non-empty substring"))
    } else {
        Ok(value)
    }
}

fn parse_due(value: &str) -> Result<Due, FilterError> {
    let parsed = match value.to_ascii_lowercase().as_str() {
        "today" => Some(Due::Today),
        "overdue" => Some(Due::Overdue),
        "week" => Some(Due::Week),
        "none" => Some(Due::None),
        _ => value.parse().ok().map(Due::Exact),
    };
    parsed.ok_or_else(|| invalid_value("due", value, "today, overdue, week, none, or YYYY-MM-DD"))
}

fn parse_priority(value: &str) -> Result<Option<Priority>, FilterError> {
    match value.to_ascii_lowercase().as_str() {
        "high" => Ok(Some(Priority::High)),
        "medium" => Ok(Some(Priority::Medium)),
        "low" => Ok(Some(Priority::Low)),
        "none" => Ok(None),
        _ => Err(invalid_value("pri", value, "high, medium, low, or none")),
    }
}

fn parse_is(value: &str) -> Result<Is, FilterError> {
    match value.to_ascii_lowercase().as_str() {
        "overdue" => Ok(Is::Overdue),
        "done" => Ok(Is::Done),
        "open" => Ok(Is::Open),
        "event" => Ok(Is::Event),
        _ => Err(invalid_value("is", value, "overdue, done, open, or event")),
    }
}

fn invalid_value(field: &'static str, value: &str, expected: &'static str) -> FilterError {
    FilterError::InvalidValue {
        field,
        value: value.into(),
        expected,
    }
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

fn fuzzy_contains(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.to_lowercase();
    haystack.contains(needle)
        || needle
            .chars()
            .try_fold(haystack.chars(), |mut characters, needle| {
                characters
                    .find(|haystack| *haystack == needle)
                    .map(|_| characters)
            })
            .is_some()
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 30);

    fn reminder<'a>(title: &'a str, list: &'a str) -> FilterRow<'a> {
        FilterRow {
            kind: FilterKind::Reminder,
            title,
            collection: list,
            due: Some(TODAY),
            priority: Some(Priority::High),
            completed: false,
        }
    }

    fn event<'a>(title: &'a str, calendar: &'a str) -> FilterRow<'a> {
        FilterRow {
            kind: FilterKind::Event,
            title,
            collection: calendar,
            due: Some(TODAY),
            priority: None,
            completed: false,
        }
    }

    fn matches(expression: &str, row: &FilterRow<'_>) -> bool {
        Filter::parse(expression).unwrap().matches(row, TODAY)
    }

    #[test]
    fn list_and_calendar_fields_match_their_own_kind_by_substring() {
        assert!(matches("list:PROJ", &reminder("Review", "Work Projects")));
        assert!(!matches("list:work", &event("Review", "Work")));
        assert!(matches("cal:wOr", &event("Planning", "Work Calendar")));
        assert!(!matches("cal:work", &reminder("Planning", "Work")));
        assert!(!matches("list:wprj", &reminder("Review", "Work Projects")));
    }

    #[test]
    fn due_field_supports_every_value() {
        let cases = [
            ("due:today", Some(TODAY), true),
            ("due:overdue", Some(date(2026, 10, 29)), true),
            ("due:week", Some(date(2026, 10, 29)), true),
            ("due:week", Some(date(2026, 11, 6)), true),
            ("due:week", Some(date(2026, 11, 7)), false),
            ("due:none", None, true),
            ("due:2026-11-04", Some(date(2026, 11, 4)), true),
            ("due:2026-11-04", Some(TODAY), false),
        ];
        for (expression, due, expected) in cases {
            let mut row = reminder("Review", "Work");
            row.due = due;
            assert_eq!(matches(expression, &row), expected, "{expression}: {due:?}");
        }
    }

    #[test]
    fn priority_field_supports_every_value() {
        for (value, priority) in [
            ("high", Some(Priority::High)),
            ("medium", Some(Priority::Medium)),
            ("low", Some(Priority::Low)),
            ("none", None),
        ] {
            let mut row = reminder("Review", "Work");
            row.priority = priority;
            assert!(matches(&format!("pri:{value}"), &row));
        }
    }

    #[test]
    fn is_field_supports_every_value() {
        let mut overdue = reminder("Late", "Work");
        overdue.due = Some(date(2026, 10, 29));
        assert!(matches("is:overdue", &overdue));
        assert!(matches("is:open", &overdue));

        let mut done = reminder("Filed", "Work");
        done.completed = true;
        assert!(matches("is:done", &done));
        assert!(matches("is:event", &event("Planning", "Work")));
        assert!(!matches("is:event", &overdue));
    }

    #[test]
    fn tokens_are_anded_and_can_be_negated() {
        let row = reminder("Review pull request", "Work");
        assert!(matches("review list:work !pri:low", &row));
        assert!(!matches("review list:home", &row));
        assert!(!matches("!review", &row));
    }

    #[test]
    fn free_text_accepts_case_insensitive_substrings_and_subsequences() {
        let row = reminder("Review Pull Request", "Engineering Projects");
        assert!(matches("PULL", &row));
        assert!(matches("rvw", &row));
        assert!(matches("engprj", &row));
        assert!(!matches("wvr", &row));
    }

    #[test]
    fn unknown_fields_are_free_text() {
        assert!(matches(
            "http://example.com",
            &reminder("Visit HTTP://example.com", "Inbox")
        ));
        assert!(matches(
            "owner:alice",
            &reminder("Ask owner:alice", "Inbox")
        ));
    }

    #[test]
    fn known_fields_reject_unknown_or_empty_values() {
        let cases = [
            ("due:tomorrow", "today, overdue, week, none, or YYYY-MM-DD"),
            (
                "due:2026-02-30",
                "today, overdue, week, none, or YYYY-MM-DD",
            ),
            ("pri:urgent", "high, medium, low, or none"),
            ("is:reminder", "overdue, done, open, or event"),
            ("list:", "a non-empty substring"),
            ("cal:", "a non-empty substring"),
        ];
        for (expression, expected) in cases {
            let error = Filter::parse(expression).unwrap_err();
            assert_eq!(
                error.code().unwrap().to_string(),
                "nib::filter::invalid_value"
            );
            assert!(
                error.to_string().contains(expected),
                "{expression}: {error}"
            );
        }
        let error = Filter::parse("!").unwrap_err();
        assert_eq!(
            error.code().unwrap().to_string(),
            "nib::filter::empty_token"
        );
    }

    #[test]
    fn completed_items_require_a_positive_done_clause() {
        let mut row = reminder("Filed report", "Work");
        row.completed = true;
        assert!(!matches("", &row));
        assert!(!matches("filed", &row));
        assert!(!matches("!is:done", &row));
        assert!(matches("is:done filed", &row));
        assert!(!matches("is:done !filed", &row));
    }
}

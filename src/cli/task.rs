use jiff::Span;
use jiff::civil::{Date, Weekday};
use thiserror::Error;

use crate::domain::Priority;

const DUE_EXPECTED: &str =
    "expected today, tomorrow, a weekday, next WEEKDAY, in Nd, in Nw, +Nd, +Nw, eom, or YYYY-MM-DD";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Due {
    Today,
    Tomorrow,
    Weekday { weekday: Weekday, next: bool },
    Offset { count: i64, unit: OffsetUnit },
    EndOfMonth,
    On(Date),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OffsetUnit {
    Day,
    Week,
}

impl Due {
    /// Resolves this expression relative to the injected local date.
    ///
    /// A bare weekday is the nearest matching date on or after `today`, so it
    /// resolves to `today` when the weekdays match. `next WEEKDAY` skips that
    /// occurrence and resolves exactly seven days after the bare weekday.
    pub(super) fn on(self, today: Date) -> Result<Date, jiff::Error> {
        match self {
            Self::Today => Ok(today),
            Self::Tomorrow => today.tomorrow(),
            Self::Weekday { weekday, next } => {
                let days = i64::from(today.weekday().until(weekday)) + i64::from(next) * 7;
                today.checked_add(Span::new().try_days(days)?)
            }
            Self::Offset { count, unit } => {
                let span = match unit {
                    OffsetUnit::Day => Span::new().try_days(count)?,
                    OffsetUnit::Week => Span::new().try_weeks(count)?,
                };
                today.checked_add(span)
            }
            Self::EndOfMonth => Date::new(today.year(), today.month(), today.days_in_month()),
            Self::On(date) => Ok(date),
        }
    }
}

pub(super) fn due(value: &str) -> Result<Due, String> {
    let words: Vec<&str> = value.split_whitespace().collect();
    match words.as_slice() {
        [word] => parse_single_due(word)?.ok_or_else(|| DUE_EXPECTED.into()),
        [prefix, weekday] if prefix.eq_ignore_ascii_case("next") => parse_weekday(weekday)
            .map(|weekday| Due::Weekday {
                weekday,
                next: true,
            })
            .ok_or_else(|| "expected a weekday after `next`".into()),
        [prefix, interval] if prefix.eq_ignore_ascii_case("in") => parse_interval(interval)?
            .ok_or_else(|| "expected a number followed by `d` or `w` after `in`".into()),
        _ => Err(DUE_EXPECTED.into()),
    }
}

fn parse_single_due(value: &str) -> Result<Option<Due>, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "today" => Ok(Some(Due::Today)),
        "tomorrow" => Ok(Some(Due::Tomorrow)),
        "eom" => Ok(Some(Due::EndOfMonth)),
        _ if is_iso_date(value) => value
            .parse()
            .map(Due::On)
            .map(Some)
            .map_err(|_| "that date doesn't exist".into()),
        _ if looks_like_iso_date(value) => Err("expected YYYY-MM-DD".into()),
        _ if value.starts_with('+') => parse_interval(&value[1..])?
            .map(Some)
            .ok_or_else(|| "expected a number followed by `d` or `w` after `+`".into()),
        _ => Ok(parse_weekday(&lower).map(|weekday| Due::Weekday {
            weekday,
            next: false,
        })),
    }
}

fn parse_interval(value: &str) -> Result<Option<Due>, String> {
    let Some((unit, count)) = value
        .as_bytes()
        .split_last()
        .map(|(unit, count)| (*unit, count))
    else {
        return Ok(None);
    };
    let unit = match unit.to_ascii_lowercase() {
        b'd' => OffsetUnit::Day,
        b'w' => OffsetUnit::Week,
        _ => return Ok(None),
    };
    if count.is_empty() || !count.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    let count = std::str::from_utf8(count)
        .map_err(|_| "relative date must use ASCII digits".to_owned())?
        .parse()
        .map_err(|_| "relative date is too large".to_owned())?;
    Ok(Some(Due::Offset { count, unit }))
}

fn parse_weekday(value: &str) -> Option<Weekday> {
    match value.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Monday),
        "tue" | "tuesday" => Some(Weekday::Tuesday),
        "wed" | "wednesday" => Some(Weekday::Wednesday),
        "thu" | "thursday" => Some(Weekday::Thursday),
        "fri" | "friday" => Some(Weekday::Friday),
        "sat" | "saturday" => Some(Weekday::Saturday),
        "sun" | "sunday" => Some(Weekday::Sunday),
        _ => None,
    }
}

fn is_iso_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

fn looks_like_iso_date(value: &str) -> bool {
    let mut components = value.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    let day = day.split_once('T').map_or(day, |(date, _)| date);
    year.len() == 4
        && year.bytes().all(|byte| byte.is_ascii_digit())
        && !month.is_empty()
        && month.bytes().all(|byte| byte.is_ascii_digit())
        && !day.is_empty()
        && day.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct InlineTask {
    pub title: String,
    pub list: Option<String>,
    pub due: Option<Due>,
    pub priority: Option<Priority>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum InlineError {
    #[error("inline {field} is specified more than once; escape one with a leading backslash")]
    Duplicate { field: &'static str },
    #[error("invalid inline priority {0:?}; expected !low, !medium, or !high")]
    Priority(String),
    #[error("inline reminder list can't be empty; use #NAME or escape # with a leading backslash")]
    EmptyList,
    #[error(
        "reminder title can't be empty after inline fields; escape a token with a leading backslash to keep it in the title"
    )]
    EmptyTitle,
    #[error("invalid inline due date {value:?}: {reason}")]
    Due { value: String, reason: String },
}

pub(super) fn parse_inline(words: &[String]) -> Result<InlineTask, InlineError> {
    let mut end = words.len();
    let mut list = None;
    let mut due = None;
    let mut priority = None;

    while end > 0 {
        if end >= 2 && is_escaped_due_phrase(&words[end - 2], &words[end - 1]) {
            break;
        }
        if words[end - 1].starts_with('\\') {
            break;
        }
        if end >= 2
            && words[end - 2].eq_ignore_ascii_case("next")
            && let Some(weekday) = parse_weekday(&words[end - 1])
        {
            set_once(
                &mut due,
                Due::Weekday {
                    weekday,
                    next: true,
                },
                "due date",
            )?;
            end -= 2;
            continue;
        }
        if end >= 2 && words[end - 2].eq_ignore_ascii_case("in") {
            match parse_interval(&words[end - 1]) {
                Ok(Some(value)) => {
                    set_once(&mut due, value, "due date")?;
                    end -= 2;
                    continue;
                }
                Ok(None)
                    if words[end - 1].starts_with(|character: char| character.is_ascii_digit()) =>
                {
                    return Err(InlineError::Due {
                        value: format!("in {}", words[end - 1]),
                        reason: "expected a number followed by `d` or `w` after `in`".into(),
                    });
                }
                Ok(None) => {}
                Err(reason) => {
                    return Err(InlineError::Due {
                        value: format!("in {}", words[end - 1]),
                        reason,
                    });
                }
            }
        }

        let word = &words[end - 1];
        if let Some(value) = word.strip_prefix('!') {
            let value = match value.to_ascii_lowercase().as_str() {
                "low" => Priority::Low,
                "medium" => Priority::Medium,
                "high" => Priority::High,
                _ => return Err(InlineError::Priority(word.clone())),
            };
            set_once(&mut priority, value, "priority")?;
            end -= 1;
            continue;
        }
        if let Some(value) = word.strip_prefix('#') {
            if value.is_empty() {
                return Err(InlineError::EmptyList);
            }
            set_once(&mut list, value.to_owned(), "list")?;
            end -= 1;
            continue;
        }
        match parse_single_due(word) {
            Ok(Some(value)) => {
                set_once(&mut due, value, "due date")?;
                end -= 1;
            }
            Ok(None) => break,
            Err(reason) => {
                return Err(InlineError::Due {
                    value: word.clone(),
                    reason,
                });
            }
        }
    }

    let title = inline_title(&words[..end]);
    if title.is_empty() {
        return Err(InlineError::EmptyTitle);
    }
    Ok(InlineTask {
        title,
        list,
        due,
        priority,
    })
}

fn inline_title(words: &[String]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if is_escaped_inline_word(words, index) {
                word[1..].to_owned()
            } else {
                word.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_escaped_due_phrase(first: &str, second: &str) -> bool {
    let Some(first) = first.strip_prefix('\\') else {
        return false;
    };
    (first.eq_ignore_ascii_case("next") && parse_weekday(second).is_some())
        || (first.eq_ignore_ascii_case("in") && matches!(parse_interval(second), Ok(Some(_))))
}

fn is_escaped_inline_word(words: &[String], index: usize) -> bool {
    let Some(word) = words[index].strip_prefix('\\') else {
        return false;
    };
    word.starts_with(['!', '#'])
        || matches!(parse_single_due(word), Ok(Some(_)) | Err(_))
        || words
            .get(index + 1)
            .is_some_and(|next| is_escaped_due_phrase(&words[index], next))
}

fn set_once<T>(slot: &mut Option<T>, value: T, field: &'static str) -> Result<(), InlineError> {
    if slot.is_some() {
        return Err(InlineError::Duplicate { field });
    }
    *slot = Some(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn every_due_form_resolves_against_the_supplied_date() {
        let cases = [
            ("today", date(2026, 10, 26)),
            ("tomorrow", date(2026, 10, 27)),
            ("fri", date(2026, 10, 30)),
            ("friday", date(2026, 10, 30)),
            ("next fri", date(2026, 11, 6)),
            ("in 3d", date(2026, 10, 29)),
            ("in 2w", date(2026, 11, 9)),
            ("+2w", date(2026, 11, 9)),
            ("eom", date(2026, 10, 31)),
            ("2027-01-05", date(2027, 1, 5)),
        ];
        for (input, expected) in cases {
            assert_eq!(
                due(input).unwrap().on(date(2026, 10, 26)).unwrap(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn weekday_and_next_semantics_include_then_skip_today() {
        let friday = date(2026, 10, 30);
        assert_eq!(due("fri").unwrap().on(friday).unwrap(), friday);
        assert_eq!(
            due("next friday").unwrap().on(friday).unwrap(),
            date(2026, 11, 6)
        );
    }

    #[test]
    fn weekday_names_accept_short_and_full_forms() {
        let cases = [
            ("mon", "monday", date(2026, 10, 26)),
            ("tue", "tuesday", date(2026, 10, 27)),
            ("wed", "wednesday", date(2026, 10, 28)),
            ("thu", "thursday", date(2026, 10, 29)),
            ("fri", "friday", date(2026, 10, 30)),
            ("sat", "saturday", date(2026, 10, 31)),
            ("sun", "sunday", date(2026, 11, 1)),
        ];
        for (short, full, expected) in cases {
            for input in [short, full] {
                assert_eq!(
                    due(input).unwrap().on(date(2026, 10, 26)).unwrap(),
                    expected,
                    "{input}"
                );
            }
        }
    }

    #[test]
    fn inline_fields_are_parsed_only_from_the_suffix() {
        let cases = [
            (
                &["Pay", "rent", "fri", "!high", "#bills"][..],
                "Pay rent",
                Some("bills"),
                Some(Due::Weekday {
                    weekday: Weekday::Friday,
                    next: false,
                }),
                Some(Priority::High),
            ),
            (
                &["File", "report", "next", "fri"][..],
                "File report",
                None,
                Some(Due::Weekday {
                    weekday: Weekday::Friday,
                    next: true,
                }),
                None,
            ),
            (
                &["Check", "in", "3d"][..],
                "Check",
                None,
                Some(Due::Offset {
                    count: 3,
                    unit: OffsetUnit::Day,
                }),
                None,
            ),
            (
                &["Discuss", "fri", "tomorrow", "morning"][..],
                "Discuss fri tomorrow morning",
                None,
                None,
                None,
            ),
            (
                &["Pay", "fri", "at", "noon"][..],
                "Pay fri at noon",
                None,
                None,
                None,
            ),
        ];
        for (words, title, list, due, priority) in cases {
            let words: Vec<String> = words.iter().map(ToString::to_string).collect();
            let parsed = parse_inline(&words).unwrap();
            assert_eq!(parsed.title, title, "{words:?}");
            assert_eq!(parsed.list.as_deref(), list, "{words:?}");
            assert_eq!(parsed.due, due, "{words:?}");
            assert_eq!(parsed.priority, priority, "{words:?}");
        }
    }

    #[test]
    fn backslashes_keep_inline_tokens_in_the_title() {
        let cases = [
            (&["Pay", "\\fri"][..], "Pay fri"),
            (&["Choose", "\\!high"][..], "Choose !high"),
            (&["Tag", "\\#bills"][..], "Tag #bills"),
            (&["Pay", "\\fri", "#bills"][..], "Pay fri"),
        ];
        for (words, title) in cases {
            let words: Vec<String> = words.iter().map(ToString::to_string).collect();
            let parsed = parse_inline(&words).unwrap();
            assert_eq!(parsed.title, title, "{words:?}");
        }
    }

    #[test]
    fn escaping_is_phrase_aware_and_preserves_unrelated_backslashes() {
        let cases = [
            (&["Discuss", "\\next", "fri"][..], "Discuss next fri"),
            (&["Review", "\\in", "3d"][..], "Review in 3d"),
            (&["Copy", "\\server", "docs"][..], "Copy \\server docs"),
            (
                &["Discuss", "\\next", "fri", "#work"][..],
                "Discuss next fri",
            ),
        ];
        for (words, title) in cases {
            let words: Vec<String> = words.iter().map(ToString::to_string).collect();
            let parsed = parse_inline(&words).unwrap();
            assert_eq!(parsed.title, title, "{words:?}");
        }
    }

    #[test]
    fn non_date_hyphenated_year_suffix_stays_in_the_title() {
        let words = ["Renew".into(), "2026-plan".into()];
        let parsed = parse_inline(&words).unwrap();
        assert_eq!(parsed.title, "Renew 2026-plan");
        assert_eq!(parsed.due, None);
    }

    #[test]
    fn ambiguous_and_invalid_inline_fields_are_errors() {
        let cases = [
            (
                &["Pay", "today", "tomorrow"][..],
                "due date is specified more than once",
            ),
            (
                &["Pay", "!low", "!high"][..],
                "priority is specified more than once",
            ),
            (
                &["Pay", "#home", "#work"][..],
                "list is specified more than once",
            ),
            (&["Pay", "!urgent"][..], "invalid inline priority"),
            (&["Pay", "#"][..], "list can't be empty"),
            (&["Pay", "2026-1-5"][..], "expected YYYY-MM-DD"),
            (&["Pay", "in", "3x"][..], "expected a number"),
            (&["today"][..], "title can't be empty"),
        ];
        for (words, message) in cases {
            let words: Vec<String> = words.iter().map(ToString::to_string).collect();
            let error = parse_inline(&words).unwrap_err();
            assert!(error.to_string().contains(message), "{words:?}: {error}");
        }
    }
}

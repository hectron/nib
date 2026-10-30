use jiff::Span;
use jiff::civil::{Date, Time, Weekday};
use thiserror::Error;

use crate::domain::Priority;

const DUE_EXPECTED: &str = "expected today, tomorrow, a weekday, next WEEKDAY, in Nd, in Nw, +Nd, +Nw, eom, YYYY-MM-DD, a time, or a date followed by a time";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Due {
    date: Option<DueDate>,
    time: Option<Time>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DueDate {
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

impl DueDate {
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

impl Due {
    pub(super) fn on(self, today: Date) -> Result<(Date, Option<Time>), jiff::Error> {
        Ok((
            self.date.map_or(Ok(today), |date| date.on(today))?,
            self.time,
        ))
    }
}

pub(super) fn due(value: &str) -> Result<Due, String> {
    if let Some((date, time)) = value
        .split_once('T')
        .filter(|(date, _)| is_iso_date(date) || looks_like_iso_date(date))
    {
        let date = parse_single_due(date)?.ok_or_else(|| DUE_EXPECTED.to_owned())?;
        let time = parse_time(time)?.ok_or_else(|| "expected a time after `T`".to_owned())?;
        return Ok(Due {
            date: Some(date),
            time: Some(time),
        });
    }
    let words: Vec<&str> = value.split_whitespace().collect();
    let (date, time) = match words.as_slice() {
        [word] => {
            if let Some(time) = parse_time(word)? {
                (None, Some(time))
            } else {
                (
                    Some(parse_single_due(word)?.ok_or_else(|| DUE_EXPECTED.to_owned())?),
                    None,
                )
            }
        }
        [prefix, value] => {
            if let Some(time) = parse_time(value)? {
                (
                    Some(parse_single_due(prefix)?.ok_or_else(|| DUE_EXPECTED.to_owned())?),
                    Some(time),
                )
            } else if prefix.eq_ignore_ascii_case("next") {
                (
                    Some(
                        parse_weekday(value)
                            .map(|weekday| DueDate::Weekday {
                                weekday,
                                next: true,
                            })
                            .ok_or_else(|| "expected a weekday after `next`".to_owned())?,
                    ),
                    None,
                )
            } else if prefix.eq_ignore_ascii_case("in") {
                (
                    Some(parse_interval(value)?.ok_or_else(|| {
                        "expected a number followed by `d` or `w` after `in`".to_owned()
                    })?),
                    None,
                )
            } else {
                return Err(DUE_EXPECTED.into());
            }
        }
        [prefix, value, time] if parse_time(time)?.is_some() => {
            let phrase = format!("{prefix} {value}");
            let parsed = due(&phrase)?;
            (parsed.date, parse_time(time)?)
        }
        _ => return Err(DUE_EXPECTED.into()),
    };
    Ok(Due { date, time })
}

fn parse_single_due(value: &str) -> Result<Option<DueDate>, String> {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "today" => Ok(Some(DueDate::Today)),
        "tomorrow" => Ok(Some(DueDate::Tomorrow)),
        "eom" => Ok(Some(DueDate::EndOfMonth)),
        _ if is_iso_date(value) => value
            .parse()
            .map(DueDate::On)
            .map(Some)
            .map_err(|_| "that date doesn't exist".into()),
        _ if looks_like_iso_date(value) => Err("expected YYYY-MM-DD".into()),
        _ if value.starts_with('+') => parse_interval(&value[1..])?
            .map(Some)
            .ok_or_else(|| "expected a number followed by `d` or `w` after `+`".into()),
        _ => Ok(parse_weekday(&lower).map(|weekday| DueDate::Weekday {
            weekday,
            next: false,
        })),
    }
}

fn parse_interval(value: &str) -> Result<Option<DueDate>, String> {
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
    Ok(Some(DueDate::Offset { count, unit }))
}

fn parse_time(value: &str) -> Result<Option<Time>, String> {
    let lower = value.to_ascii_lowercase();
    if let Some(digits) = lower
        .strip_suffix("am")
        .or_else(|| lower.strip_suffix("pm"))
    {
        let pm = lower.ends_with("pm");
        let (hour, minute) = match digits.split_once(':') {
            Some((hour, minute)) => (hour, minute),
            None => (digits, "0"),
        };
        if hour.is_empty()
            || hour.len() > 2
            || minute.is_empty()
            || minute.len() > 2
            || !hour.bytes().all(|byte| byte.is_ascii_digit())
            || !minute.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok(None);
        }
        let hour: i8 = hour.parse().map_err(|_| "invalid hour".to_owned())?;
        let minute: i8 = minute.parse().map_err(|_| "invalid minute".to_owned())?;
        if !(1..=12).contains(&hour) || !(0..=59).contains(&minute) {
            return Err("that time doesn't exist".into());
        }
        let hour = if pm { hour % 12 + 12 } else { hour % 12 };
        return Time::new(hour, minute, 0, 0)
            .map(Some)
            .map_err(|_| "that time doesn't exist".into());
    }
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return Ok(None);
    }
    if !value
        .bytes()
        .enumerate()
        .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
    {
        return Ok(None);
    }
    value
        .parse::<Time>()
        .map(Some)
        .map_err(|_| "that time doesn't exist".into())
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
    let (list, due, priority) = inline_fields(words, &mut end, true)?;
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

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(super) fn inline_title_len(words: &[String]) -> usize {
    let mut end = words.len();
    let _ = inline_fields(words, &mut end, false);
    end
}

type InlineFields = (Option<String>, Option<Due>, Option<Priority>);

fn inline_fields(
    words: &[String],
    end: &mut usize,
    reject_duplicates: bool,
) -> Result<InlineFields, InlineError> {
    let mut list = None;
    let mut due = None;
    let mut priority = None;
    let set_once = |result: Result<(), InlineError>| {
        if reject_duplicates { result } else { Ok(()) }
    };

    while *end > 0 {
        let end_now = *end;
        if end_now >= 2 && is_escaped_due_phrase(&words[end_now - 2], &words[end_now - 1]) {
            break;
        }
        if words[end_now - 1].starts_with('\\') {
            break;
        }
        let word = &words[end_now - 1];
        if let Some(value) = word.strip_prefix('!') {
            let value = match value.to_ascii_lowercase().as_str() {
                "low" => Priority::Low,
                "medium" => Priority::Medium,
                "high" => Priority::High,
                _ => return Err(InlineError::Priority(word.clone())),
            };
            set_once(set_field(&mut priority, value, "priority"))?;
            *end -= 1;
            continue;
        }
        if let Some(value) = word.strip_prefix('#') {
            if value.is_empty() {
                return Err(InlineError::EmptyList);
            }
            set_once(set_field(&mut list, value.to_owned(), "list"))?;
            *end -= 1;
            continue;
        }
        match inline_due_suffix(&words[..end_now]) {
            Ok(Some((value, count))) => {
                set_once(set_field(&mut due, value, "due date"))?;
                *end -= count;
            }
            Ok(None) => break,
            Err((value, reason)) => return Err(InlineError::Due { value, reason }),
        }
    }

    Ok((list, due, priority))
}

fn inline_due_suffix(words: &[String]) -> Result<Option<(Due, usize)>, (String, String)> {
    let last = words
        .last()
        .expect("inline parsing only checks a non-empty suffix");
    if looks_like_iso_datetime(last) {
        return due(last)
            .map(|value| Some((value, 1)))
            .map_err(|reason| (last.clone(), reason));
    }
    let time = parse_time(last).map_err(|reason| (last.clone(), reason))?;
    if time.is_some() {
        if words.len() >= 3
            && is_escaped_due_phrase(&words[words.len() - 3], &words[words.len() - 2])
        {
            return Ok(None);
        }
        if words.len() >= 3 {
            let phrase = format!(
                "{} {} {}",
                words[words.len() - 3],
                words[words.len() - 2],
                last
            );
            if matches!(
                words[words.len() - 3].to_ascii_lowercase().as_str(),
                "next" | "in"
            ) {
                match due(&phrase) {
                    Ok(value) => return Ok(Some((value, 3))),
                    Err(reason)
                        if words[words.len() - 3].eq_ignore_ascii_case("in")
                            && words[words.len() - 2]
                                .starts_with(|character: char| character.is_ascii_digit()) =>
                    {
                        return Err((phrase, reason));
                    }
                    Err(_) => {}
                }
            }
        }
        if words.len() >= 2 && !words[words.len() - 2].starts_with('\\') {
            let phrase = format!("{} {last}", words[words.len() - 2]);
            if matches!(parse_single_due(&words[words.len() - 2]), Ok(Some(_))) {
                return due(&phrase)
                    .map(|value| Some((value, 2)))
                    .map_err(|reason| (phrase, reason));
            }
        }
        return due(last)
            .map(|value| Some((value, 1)))
            .map_err(|reason| (last.clone(), reason));
    }
    if words.len() >= 2
        && matches!(
            words[words.len() - 2].to_ascii_lowercase().as_str(),
            "next" | "in"
        )
    {
        let phrase = format!("{} {last}", words[words.len() - 2]);
        return match due(&phrase) {
            Ok(value) => Ok(Some((value, 2))),
            Err(reason)
                if words[words.len() - 2].eq_ignore_ascii_case("in")
                    && last.starts_with(|character: char| character.is_ascii_digit()) =>
            {
                Err((phrase, reason))
            }
            Err(_) => match parse_single_due(last) {
                Ok(Some(date)) => Ok(Some((
                    Due {
                        date: Some(date),
                        time: None,
                    },
                    1,
                ))),
                Ok(None) => Ok(None),
                Err(reason) => Err((last.clone(), reason)),
            },
        };
    }
    match parse_single_due(last) {
        Ok(Some(date)) => Ok(Some((
            Due {
                date: Some(date),
                time: None,
            },
            1,
        ))),
        Ok(None) => Ok(None),
        Err(reason) => Err((last.clone(), reason)),
    }
}

fn looks_like_iso_datetime(value: &str) -> bool {
    value
        .split_once('T')
        .is_some_and(|(date, _)| is_iso_date(date) || looks_like_iso_date(date))
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
        || (matches!(parse_single_due(first), Ok(Some(_)))
            && matches!(parse_time(second), Ok(Some(_))))
}

fn is_escaped_inline_word(words: &[String], index: usize) -> bool {
    let Some(word) = words[index].strip_prefix('\\') else {
        return false;
    };
    word.starts_with(['!', '#'])
        || matches!(parse_single_due(word), Ok(Some(_)) | Err(_))
        || matches!(parse_time(word), Ok(Some(_)) | Err(_))
        || word.contains('T') && due(word).is_ok()
        || words
            .get(index + 1)
            .is_some_and(|next| is_escaped_due_phrase(&words[index], next))
}

fn set_field<T>(slot: &mut Option<T>, value: T, field: &'static str) -> Result<(), InlineError> {
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
                due(input).unwrap().on(date(2026, 10, 26)).unwrap().0,
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn due_times_accept_24_hour_and_twelve_hour_forms() {
        let cases = [
            (
                "tomorrow 09:00",
                date(2026, 10, 27),
                Time::new(9, 0, 0, 0).unwrap(),
            ),
            (
                "fri 9am",
                date(2026, 10, 30),
                Time::new(9, 0, 0, 0).unwrap(),
            ),
            (
                "9:30pm",
                date(2026, 10, 26),
                Time::new(21, 30, 0, 0).unwrap(),
            ),
            (
                "2026-11-03T09:00",
                date(2026, 11, 3),
                Time::new(9, 0, 0, 0).unwrap(),
            ),
            (
                "2026-11-03 09:00",
                date(2026, 11, 3),
                Time::new(9, 0, 0, 0).unwrap(),
            ),
        ];
        for (input, expected_date, time) in cases {
            assert_eq!(
                due(input).unwrap().on(date(2026, 10, 26)).unwrap(),
                (expected_date, Some(time)),
                "{input}"
            );
        }
    }

    #[test]
    fn date_words_stay_case_insensitive_with_times() {
        let cases = ["Tomorrow", "Thu", "Tuesday", "next Tue"];
        for input in cases {
            assert!(due(input).is_ok(), "{input}");
            assert!(due(&format!("{input} 9am")).is_ok(), "{input} 9am");
        }
    }

    #[test]
    fn inline_times_must_follow_a_date_or_stand_alone() {
        let cases = [
            (&["Pay", "fri", "9am"][..], "Pay", date(2026, 10, 30), 9, 0),
            (&["Call", "9:30pm"][..], "Call", date(2026, 10, 26), 21, 30),
            (
                &["Pay", "fri", "later", "09:00"][..],
                "Pay fri later",
                date(2026, 10, 26),
                9,
                0,
            ),
            (
                &["Deploy", "2026-11-03T09:00"][..],
                "Deploy",
                date(2026, 11, 3),
                9,
                0,
            ),
        ];
        for (words, title, expected_date, hour, minute) in cases {
            let words = words.iter().map(ToString::to_string).collect::<Vec<_>>();
            let parsed = parse_inline(&words).unwrap();
            assert_eq!(parsed.title, title);
            assert_eq!(
                parsed.due.unwrap().on(date(2026, 10, 26)).unwrap(),
                (expected_date, Some(Time::new(hour, minute, 0, 0).unwrap()))
            );
        }
    }

    #[test]
    fn weekday_and_next_semantics_include_then_skip_today() {
        let friday = date(2026, 10, 30);
        assert_eq!(due("fri").unwrap().on(friday).unwrap().0, friday);
        assert_eq!(
            due("next friday").unwrap().on(friday).unwrap().0,
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
                    due(input).unwrap().on(date(2026, 10, 26)).unwrap().0,
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
                Some(Due {
                    date: Some(DueDate::Weekday {
                        weekday: Weekday::Friday,
                        next: false,
                    }),
                    time: None,
                }),
                Some(Priority::High),
            ),
            (
                &["File", "report", "next", "fri"][..],
                "File report",
                None,
                Some(Due {
                    date: Some(DueDate::Weekday {
                        weekday: Weekday::Friday,
                        next: true,
                    }),
                    time: None,
                }),
                None,
            ),
            (
                &["Check", "in", "3d"][..],
                "Check",
                None,
                Some(Due {
                    date: Some(DueDate::Offset {
                        count: 3,
                        unit: OffsetUnit::Day,
                    }),
                    time: None,
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
    fn invalid_in_and_next_phrases_fall_back_to_shorter_suffixes() {
        let cases = [
            (
                &["Check", "in", "tomorrow"][..],
                "Check in",
                date(2026, 10, 27),
                None,
            ),
            (
                &["Plan", "next", "tomorrow"][..],
                "Plan next",
                date(2026, 10, 27),
                None,
            ),
            (
                &["Check", "in", "tomorrow", "9am"][..],
                "Check in",
                date(2026, 10, 27),
                Some(Time::new(9, 0, 0, 0).unwrap()),
            ),
            (
                &["Review", "in", "person", "3pm"][..],
                "Review in person",
                date(2026, 10, 26),
                Some(Time::new(15, 0, 0, 0).unwrap()),
            ),
            (
                &["Meeting", "next", "week", "9am"][..],
                "Meeting next week",
                date(2026, 10, 26),
                Some(Time::new(9, 0, 0, 0).unwrap()),
            ),
        ];
        for (words, title, expected_date, time) in cases {
            let words = words.iter().map(ToString::to_string).collect::<Vec<_>>();
            let parsed = parse_inline(&words).unwrap();
            assert_eq!(parsed.title, title);
            assert_eq!(
                parsed.due.unwrap().on(date(2026, 10, 26)).unwrap(),
                (expected_date, time)
            );
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
            (&["Pay", "\\next", "fri", "9am"][..], "Pay next fri 9am"),
            (&["Keep", "\\in", "3d", "9am"][..], "Keep in 3d 9am"),
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
            assert_eq!(parsed.due, None, "{words:?}");
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

    #[test]
    fn the_title_length_skips_duplicate_fields_and_stops_at_a_bad_one() {
        let cases = [
            (&["Pay", "rent", "fri", "!high", "#bills"][..], 2),
            (&["!high", "Pay", "rent"][..], 3),
            (&["!high"][..], 0),
            (&["Call", "mom", "tomorrow", "fri"][..], 2),
            (&["Pay", "!low", "!high"][..], 1),
            (&["Pay", "#home", "#work"][..], 1),
            (&["Pay", "!urgent"][..], 2),
            (&["Pay", "!urgent", "!high"][..], 2),
        ];
        for (words, expected) in cases {
            let words: Vec<String> = words.iter().map(ToString::to_string).collect();
            assert_eq!(inline_title_len(&words), expected, "{words:?}");
        }
    }
}

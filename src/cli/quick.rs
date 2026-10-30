use clap::{Args, ValueEnum};

use super::{Action, word};
use crate::domain::{Priority, TaskChange, TaskChangeRequest};

#[derive(Debug, Args)]
pub(super) struct Selectors {
    /// Reminder id, or part of a title that matches exactly one reminder
    #[arg(value_name = "SELECTOR", required = true, value_parser = word)]
    selectors: Vec<String>,
    /// Only match titles in this list
    #[arg(short, long, value_name = "NAME")]
    list: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum PriorityLevel {
    High,
    Medium,
    Low,
    None,
}

impl PriorityLevel {
    fn priority(self) -> Option<Priority> {
        match self {
            Self::High => Some(Priority::High),
            Self::Medium => Some(Priority::Medium),
            Self::Low => Some(Priority::Low),
            Self::None => None,
        }
    }
}

pub(super) fn done(targets: Selectors) -> Action {
    action(TaskChange::Complete, targets)
}

pub(super) fn reopen(targets: Selectors) -> Action {
    action(TaskChange::Reopen, targets)
}

pub(super) fn priority(level: PriorityLevel, targets: Selectors) -> Action {
    action(TaskChange::Priority(level.priority()), targets)
}

pub(super) fn move_to(list: String, targets: Selectors) -> Action {
    action(TaskChange::Move(list), targets)
}

fn action(change: TaskChange, targets: Selectors) -> Action {
    Action::ChangeTasks(TaskChangeRequest {
        change,
        selectors: targets.selectors,
        list: targets.list,
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::cli::Cli;
    use crate::domain::Config;

    fn parse(args: &[&str]) -> Result<Action, clap::Error> {
        let now = date(2026, 10, 26)
            .at(9, 30, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        Cli::try_parse_from(["nib"].iter().chain(args)).map(|cli| {
            cli.command
                .into_action(&now, &Config::default())
                .unwrap()
                .unwrap()
        })
    }

    fn change(change: TaskChange, selectors: &[&str], list: Option<&str>) -> Action {
        Action::ChangeTasks(TaskChangeRequest {
            change,
            selectors: selectors.iter().map(ToString::to_string).collect(),
            list: list.map(Into::into),
        })
    }

    #[test]
    fn each_command_resolves_to_a_task_change() {
        let cases: [(&[&str], Action); 6] = [
            (
                &["task", "done", "oat milk", "rem-2"],
                change(TaskChange::Complete, &["oat milk", "rem-2"], None),
            ),
            (
                &["todo", "reopen", "-l", "Home", "dentist"],
                change(TaskChange::Reopen, &["dentist"], Some("Home")),
            ),
            (
                &["task", "priority", "high", "report"],
                change(
                    TaskChange::Priority(Some(Priority::High)),
                    &["report"],
                    None,
                ),
            ),
            (
                &["task", "priority", "none", "report", "--list", "Work"],
                change(TaskChange::Priority(None), &["report"], Some("Work")),
            ),
            (
                &["task", "move", "Work", "eggs", "oat"],
                change(TaskChange::Move("Work".into()), &["eggs", "oat"], None),
            ),
            (
                &["--json", "task", "move", "Work", "eggs"],
                change(TaskChange::Move("Work".into()), &["eggs"], None),
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(parse(args).unwrap(), expected, "nib {}", args.join(" "));
        }
    }

    #[test]
    fn selectors_are_required_and_levels_are_checked() {
        for args in [
            &["task", "done"][..],
            &["task", "reopen"],
            &["task", "priority", "high"],
            &["task", "priority", "urgent", "report"],
            &["task", "move", "Work"],
            &["task", "done", " "],
        ] {
            assert!(parse(args).is_err(), "nib {}", args.join(" "));
        }
    }
}

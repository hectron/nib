use std::path::PathBuf;
use std::time::Duration;

use crate::domain::{AutomationStatus, DoctorProbe, EditorSetting, PermissionStatus};

const MINIMUM_MACOS_MAJOR: u64 = 26;
const STALE_CONFIG_LOCK_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Warning,
    Fail,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warning => "warning",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileCheck {
    pub path: PathBuf,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLockFile {
    pub path: PathBuf,
    pub age: Duration,
}

pub fn checks(
    probe: &dyn DoctorProbe,
    config_files: &[ConfigFileCheck],
    config_error: Option<&str>,
    config_locks: &[ConfigLockFile],
) -> Vec<Check> {
    let terminal = probe.terminal();
    let mut checks = vec![macos(probe.macos_version()), terminal_check(&terminal)];
    checks.push(permission(
        "reminders",
        "Reminders",
        probe.reminders_authorization(),
        &terminal.0,
    ));
    checks.push(permission(
        "calendars",
        "Calendars",
        probe.calendars_authorization(),
        &terminal.0,
    ));
    checks.push(notes(probe.notes_automation(), &terminal.0));
    checks.extend(configuration(config_files, config_error));
    checks.push(config_lock(config_locks));
    checks.push(editor(probe.editor()));
    checks
}

fn config_lock(locks: &[ConfigLockFile]) -> Check {
    if locks.is_empty() {
        return Check {
            id: "config_lock".into(),
            status: CheckStatus::Pass,
            message: "no configuration lock files are left behind".into(),
            fix: None,
        };
    }
    let (stale, held): (Vec<&ConfigLockFile>, Vec<&ConfigLockFile>) = locks
        .iter()
        .partition(|lock| lock.age > STALE_CONFIG_LOCK_AFTER);
    let held = lock_paths(&held);
    if stale.is_empty() {
        Check {
            id: "config_lock".into(),
            status: CheckStatus::Warning,
            message: format!("configuration lock is held by another nib config command: {held}"),
            fix: Some("Wait for that command to finish, then run `nib doctor` again.".into()),
        }
    } else {
        let stale = lock_paths(&stale);
        let message = if held.is_empty() {
            format!("stale configuration lock: {stale}")
        } else {
            format!("stale configuration lock: {stale}; held by another nib config command: {held}")
        };
        Check {
            id: "config_lock".into(),
            status: CheckStatus::Fail,
            message,
            fix: Some(format!(
                "Remove {stale} if no nib config command is running."
            )),
        }
    }
}

fn lock_paths(locks: &[&ConfigLockFile]) -> String {
    locks
        .iter()
        .map(|lock| lock.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn macos(version: Result<String, String>) -> Check {
    match version {
        Ok(version) => match version
            .split('.')
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        {
            Some(major) if major >= MINIMUM_MACOS_MAJOR => Check {
                id: "macos".into(),
                status: CheckStatus::Pass,
                message: format!("macOS {version} is supported"),
                fix: None,
            },
            Some(_) => Check {
                id: "macos".into(),
                status: CheckStatus::Fail,
                message: format!("macOS {version} is too old; nib requires macOS 26 or newer"),
                fix: Some(
                    "Install macOS 26 or newer in System Settings > General > Software Update."
                        .into(),
                ),
            },
            None => failed_version(format!("couldn't parse macOS version {version:?}")),
        },
        Err(error) => failed_version(error),
    }
}

fn failed_version(error: String) -> Check {
    Check {
        id: "macos".into(),
        status: CheckStatus::Fail,
        message: error,
        fix: Some("Check the version in System Settings > General > About.".into()),
    }
}

fn terminal_check((terminal, detected): &(String, bool)) -> Check {
    if *detected {
        Check {
            id: "terminal".into(),
            status: CheckStatus::Pass,
            message: format!("macOS attributes permissions to {terminal}"),
            fix: None,
        }
    } else {
        Check {
            id: "terminal".into(),
            status: CheckStatus::Warning,
            message: "couldn't identify the terminal app that owns permissions".into(),
            fix: Some(
                "Run `nib doctor` directly in the terminal app you use for nib commands.".into(),
            ),
        }
    }
}

fn permission(id: &str, app: &'static str, status: PermissionStatus, terminal: &str) -> Check {
    let pane = format!("System Settings > Privacy & Security > {app}");
    match status {
        PermissionStatus::FullAccess => Check {
            id: id.into(),
            status: CheckStatus::Pass,
            message: format!("{terminal} has full access to {app}"),
            fix: None,
        },
        PermissionStatus::NotDetermined => Check {
            id: id.into(),
            status: CheckStatus::Warning,
            message: format!("{app} access will be requested on first use"),
            fix: Some(format!(
                "When prompted, allow {terminal}; later changes are in {pane}."
            )),
        },
        PermissionStatus::Denied => denied_permission(id, app, terminal, &pane, "denied"),
        PermissionStatus::Restricted => {
            denied_permission(id, app, terminal, &pane, "restricted by macOS")
        }
        PermissionStatus::WriteOnly => denied_permission(id, app, terminal, &pane, "write-only"),
        PermissionStatus::Unknown(detail) => Check {
            id: id.into(),
            status: CheckStatus::Fail,
            message: format!("couldn't determine {app} access: {detail}"),
            fix: Some(format!("Check {terminal} in {pane}.")),
        },
    }
}

fn denied_permission(id: &str, app: &str, terminal: &str, pane: &str, detail: &str) -> Check {
    Check {
        id: id.into(),
        status: CheckStatus::Fail,
        message: format!("{terminal} access to {app} is {detail}"),
        fix: Some(format!("Turn on {terminal} in {pane}.")),
    }
}

fn notes(status: AutomationStatus, terminal: &str) -> Check {
    let pane = "System Settings > Privacy & Security > Automation";
    match status {
        AutomationStatus::Allowed => Check {
            id: "notes".into(),
            status: CheckStatus::Pass,
            message: format!("{terminal} can automate Apple Notes"),
            fix: None,
        },
        AutomationStatus::Denied => Check {
            id: "notes".into(),
            status: CheckStatus::Fail,
            message: format!("{terminal} can't automate Apple Notes"),
            fix: Some(format!("Turn on Notes for {terminal} in {pane}.")),
        },
        AutomationStatus::NotDetermined | AutomationStatus::TargetNotRunning => Check {
            id: "notes".into(),
            status: CheckStatus::Warning,
            message: "Apple Notes Automation access is checked on first use".into(),
            fix: Some(format!(
                "When prompted, allow {terminal} to control Notes; later changes are in {pane}."
            )),
        },
        AutomationStatus::Unknown(detail) => Check {
            id: "notes".into(),
            status: CheckStatus::Fail,
            message: format!("couldn't determine Apple Notes Automation access: {detail}"),
            fix: Some(format!("Check Notes for {terminal} in {pane}.")),
        },
    }
}

fn configuration(files: &[ConfigFileCheck], error: Option<&str>) -> Vec<Check> {
    if files.is_empty() {
        return vec![Check {
            id: "config".into(),
            status: if error.is_some() {
                CheckStatus::Fail
            } else {
                CheckStatus::Pass
            },
            message: error.map_or_else(
                || "no config files loaded; built-in defaults are valid".into(),
                str::to_owned,
            ),
            fix: error
                .map(|_| "Resolve the configuration error and run `nib doctor` again.".into()),
        }];
    }
    let mut checks = files
        .iter()
        .enumerate()
        .map(|(index, file)| match &file.error {
            Some(error) => Check {
                id: format!("config_{}", index + 1),
                status: CheckStatus::Fail,
                message: error.clone(),
                fix: Some(format!(
                    "Fix {} and run `nib doctor` again.",
                    file.path.display()
                )),
            },
            None => Check {
                id: format!("config_{}", index + 1),
                status: CheckStatus::Pass,
                message: format!("config is valid: {}", file.path.display()),
                fix: None,
            },
        })
        .collect::<Vec<_>>();
    if let Some(error) = error.filter(|_| files.iter().all(|file| file.error.is_none())) {
        checks.push(Check {
            id: "config_error".into(),
            status: CheckStatus::Fail,
            message: error.into(),
            fix: Some("Resolve the configuration error and run `nib doctor` again.".into()),
        });
    }
    checks
}

fn editor(setting: EditorSetting) -> Check {
    let Some(command) = setting.command else {
        return Check {
            id: "editor".into(),
            status: CheckStatus::Warning,
            message: "$VISUAL and $EDITOR aren't set; `nib task edit` will use vi".into(),
            fix: Some("Set $VISUAL or $EDITOR to your preferred terminal editor command.".into()),
        };
    };
    let variable = setting.variable.unwrap_or("EDITOR");
    let executable = command
        .split_whitespace()
        .next()
        .and_then(|word| word.rsplit('/').next())
        .unwrap_or_default();
    let gui = matches!(executable, "code" | "subl" | "zed" | "cursor");
    let waits = command
        .split_whitespace()
        .any(|argument| matches!(argument, "--wait" | "-w"));
    if gui && !waits {
        Check {
            id: "editor".into(),
            status: CheckStatus::Warning,
            message: format!("${variable} uses `{command}` without a wait flag"),
            fix: Some(format!("Set {variable}=\"{command} --wait\".")),
        }
    } else {
        Check {
            id: "editor".into(),
            status: CheckStatus::Pass,
            message: format!("${variable} is `{command}`"),
            fix: None,
        }
    }
}

pub fn failed(checks: &[Check]) -> bool {
    checks.iter().any(|check| check.status == CheckStatus::Fail)
}

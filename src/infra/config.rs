use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use miette::Diagnostic;
use serde::Deserialize;
use thiserror::Error;

use crate::domain::{Config, Priority};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    task: Option<FileTask>,
    cal: Option<FileCal>,
    note: Option<FileNote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTask {
    list: Option<String>,
    priority: Option<Priority>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileCal {
    calendars: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileNote {
    folder: Option<String>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum ConfigError {
    #[error("couldn't read configuration file {}: {source}", path.display())]
    #[diagnostic(code(nib::config::read))]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid configuration in {}: {source}", path.display())]
    #[diagnostic(code(nib::config::invalid_file))]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("invalid value for {name}: {value:?}; expected {expected}")]
    #[diagnostic(code(nib::config::invalid_environment))]
    Environment {
        name: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("{name} contains text that isn't valid UTF-8")]
    #[diagnostic(code(nib::config::invalid_environment))]
    EnvironmentEncoding { name: &'static str },
}

pub fn load(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Config, ConfigError> {
    let mut config = Config::default();
    if let Some(path) = user_config {
        apply_if_present(&mut config, path)?;
    }

    let directories: Vec<&Path> = start.ancestors().collect();
    for directory in directories.into_iter().rev() {
        let visible = directory.join("nib.toml");
        let hidden = directory.join(".nib.toml");
        if is_distinct(&visible, user_config) && exists(&visible)? {
            apply_file(&mut config, &visible)?;
        } else if is_distinct(&hidden, user_config) && exists(&hidden)? {
            apply_file(&mut config, &hidden)?;
        }

        let local = directory.join(".nib.local.toml");
        if is_distinct(&local, user_config) {
            apply_if_present(&mut config, &local)?;
        }
    }

    apply_environment(&mut config, env)?;
    Ok(config)
}

fn is_distinct(path: &Path, user_config: Option<&Path>) -> bool {
    user_config.is_none_or(|user| path != user)
}

fn apply_if_present(config: &mut Config, path: &Path) -> Result<(), ConfigError> {
    if exists(path)? {
        apply_file(config, path)?;
    }
    Ok(())
}

fn exists(path: &Path) -> Result<bool, ConfigError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn apply_file(config: &mut Config, path: &Path) -> Result<(), ConfigError> {
    let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    let parsed: FileConfig = toml::from_str(&source).map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })?;
    apply(config, parsed);
    Ok(())
}

fn apply(config: &mut Config, other: FileConfig) {
    if let Some(task) = other.task {
        if task.list.is_some() {
            config.task.list = task.list;
        }
        if task.priority.is_some() {
            config.task.priority = task.priority;
        }
    }
    if let Some(cal) = other.cal
        && let Some(calendars) = cal.calendars
    {
        config.cal.calendars = calendars;
    }
    if let Some(note) = other.note
        && note.folder.is_some()
    {
        config.note.folder = note.folder;
    }
}

fn apply_environment(
    config: &mut Config,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<(), ConfigError> {
    if let Some(value) = environment(&env, "NIB_TASK_LIST")? {
        config.task.list = Some(value);
    }
    if let Some(value) = environment(&env, "NIB_TASK_PRIORITY")? {
        config.task.priority = Some(parse_priority("NIB_TASK_PRIORITY", &value)?);
    }
    if let Some(value) = environment(&env, "NIB_CAL_CALENDARS")? {
        config.cal.calendars = value
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
    }
    if let Some(value) = environment(&env, "NIB_NOTE_FOLDER")? {
        config.note.folder = Some(value);
    }
    Ok(())
}

fn environment(
    env: &impl Fn(&str) -> Option<OsString>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    env(name)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| ConfigError::EnvironmentEncoding { name })
        })
        .transpose()
}

fn parse_priority(name: &'static str, value: &str) -> Result<Priority, ConfigError> {
    match value {
        "low" => Ok(Priority::Low),
        "medium" => Ok(Priority::Medium),
        "high" => Ok(Priority::High),
        _ => Err(ConfigError::Environment {
            name,
            value: value.into(),
            expected: "low, medium, or high",
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use clap::Parser;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;
    use crate::cli::{Action, Cli};
    use crate::domain::{EventQuery, NewReminder};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "nib-config-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, path: impl AsRef<Path>) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn no_configuration_keeps_built_in_defaults() {
        let temp = TempDir::new();
        assert_eq!(load(&temp.0, None, no_env).unwrap(), Config::default());
    }

    #[test]
    fn nearer_files_override_farther_files_field_by_field() {
        let temp = TempDir::new();
        let project = temp.join("project");
        let package = project.join("package");
        fs::create_dir_all(&package).unwrap();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Root\"\npriority = \"low\"\n[cal]\ncalendars = [\"Home\"]\n",
        );
        write(
            &project.join(".nib.toml"),
            "[task]\nlist = \"Project\"\n[note]\nfolder = \"Work\"\n",
        );
        write(&package.join("nib.toml"), "[task]\npriority = \"high\"\n");

        let config = load(&package, None, no_env).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Project"));
        assert_eq!(config.task.priority, Some(Priority::High));
        assert_eq!(config.cal.calendars, ["Home"]);
        assert_eq!(config.note.folder.as_deref(), Some("Work"));
    }

    #[test]
    fn local_overrides_project_file() {
        let temp = TempDir::new();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Project\"\npriority = \"low\"\n",
        );
        write(&temp.join(".nib.local.toml"), "[task]\nlist = \"Local\"\n");

        let config = load(&temp.0, None, no_env).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Local"));
        assert_eq!(config.task.priority, Some(Priority::Low));
    }

    #[test]
    fn environment_overrides_files() {
        let temp = TempDir::new();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Project\"\npriority = \"low\"\n[cal]\ncalendars = [\"Home\"]\n[note]\nfolder = \"Archive\"\n",
        );
        let env = HashMap::from([
            ("NIB_TASK_LIST", "Environment"),
            ("NIB_TASK_PRIORITY", "medium"),
            ("NIB_CAL_CALENDARS", "Work, Birthdays"),
            ("NIB_NOTE_FOLDER", "Inbox"),
        ]);

        let config = load(&temp.0, None, |name| env.get(name).map(OsString::from)).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Environment"));
        assert_eq!(config.task.priority, Some(Priority::Medium));
        assert_eq!(config.cal.calendars, ["Work", "Birthdays"]);
        assert_eq!(config.note.folder.as_deref(), Some("Inbox"));
    }

    #[test]
    fn flags_override_environment_when_actions_are_built() {
        let env = HashMap::from([
            ("NIB_TASK_LIST", "Environment"),
            ("NIB_TASK_PRIORITY", "low"),
            ("NIB_CAL_CALENDARS", "Home"),
        ]);
        let config = load(Path::new("/"), None, |name| {
            env.get(name).map(OsString::from)
        })
        .unwrap();
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();

        let task = Cli::try_parse_from([
            "nib",
            "t",
            "Review",
            "PR",
            "--list",
            "Flag",
            "--priority",
            "high",
        ])
        .unwrap()
        .command
        .into_action(&now, &config)
        .unwrap();
        assert!(matches!(
            task,
            Action::AddTask(NewReminder { list: Some(list), priority: Some(Priority::High), .. })
                if list == "Flag"
        ));

        let events = Cli::try_parse_from([
            "nib",
            "cal",
            "--calendar",
            "Work",
            "--calendar",
            "Birthdays",
        ])
        .unwrap()
        .command
        .into_action(&now, &config)
        .unwrap();
        assert!(matches!(
            events,
            Action::ListEvents(EventQuery { calendars, .. })
                if calendars == ["Work", "Birthdays"]
        ));
    }

    #[test]
    fn user_config_is_not_read_again_as_a_project_file() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        write(&temp.join("nib.toml"), "[task]\nlist = \"Project\"\n");
        let user = project.join("nib.toml");
        write(&user, "[task]\nlist = \"User\"\n");

        let config = load(&project, Some(&user), no_env).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Project"));
    }

    #[test]
    fn unknown_keys_name_the_file_and_location() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        write(&path, "[task]\ncolour = \"blue\"\n");

        let error = load(&temp.0, None, no_env).unwrap_err().to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains("line 2, column 1"), "{error}");
        assert!(error.contains("unknown field `colour`"), "{error}");
    }

    #[test]
    fn invalid_environment_values_name_the_variable() {
        let error = load(Path::new("/"), None, |name| {
            (name == "NIB_TASK_PRIORITY").then(|| OsString::from("urgent"))
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("NIB_TASK_PRIORITY"), "{error}");
        assert!(error.contains("urgent"), "{error}");
    }
}

mod app;
mod cli;
mod domain;
mod infra;
#[cfg(test)]
mod test_support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jiff::Zoned;
use miette::IntoDiagnostic;

pub use cli::Cli;
pub use cli::output::Style;
pub use domain::Config;
pub use infra::config::{ConfigError, ConfigInspection};

pub struct DoctorOutput {
    pub text: String,
    pub failed: bool,
}

pub struct ConfigOutput {
    pub text: String,
    pub failed: bool,
    pub warning: Option<String>,
}

pub fn load_config(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Config, ConfigError> {
    infra::config::load(start, user_config, env)
}

pub fn inspect_config(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> ConfigInspection {
    infra::config::inspect(start, user_config, env)
}

pub fn doctor(
    inspection: &ConfigInspection,
    term_program: Option<OsString>,
    visual: Option<OsString>,
    editor: Option<OsString>,
    json: bool,
    style: Style,
) -> miette::Result<DoctorOutput> {
    let probe = infra::doctor::SystemDoctor::new(term_program, visual, editor);
    let config_files = inspection
        .files()
        .iter()
        .map(|file| app::doctor::ConfigFileCheck {
            path: file.path().to_owned(),
            error: file.error().map(str::to_owned),
        })
        .collect::<Vec<_>>();
    let config_locks = inspection
        .legacy_config_lock_files()
        .into_iter()
        .map(|path| app::doctor::ConfigLockFile { path })
        .collect::<Vec<_>>();
    let error = inspection.error().map(ToString::to_string);
    let checks = app::doctor::checks(&probe, &config_files, error.as_deref(), &config_locks);
    Ok(DoctorOutput {
        text: cli::output::doctor(&checks, json, style)?,
        failed: app::doctor::failed(&checks),
    })
}

pub fn run_config(
    cli: Cli,
    inspection: ConfigInspection,
    current_dir: &Path,
    user_config: Option<&Path>,
    style: Style,
) -> miette::Result<ConfigOutput> {
    let Some((json, action)) = cli.into_config() else {
        return Err(miette::miette!(
            "non-config command passed to the configuration entry point"
        ));
    };
    let mut failed = false;
    let mut warning = None;
    let text = match action {
        cli::ConfigAction::List { origin } => {
            let entries = inspection.into_entries()?;
            cli::output::config_list(&entries, origin, json, style)?
        }
        cli::ConfigAction::Get { key } => {
            let key = infra::config::key(&key)?;
            let entries = inspection.into_entries()?;
            let entry = entries
                .iter()
                .find(|entry| entry.key == key)
                .expect("every configuration key has an effective entry");
            if entry.value.is_unset() {
                failed = true;
                if json {
                    cli::output::config_get(entry, true, style)?
                } else {
                    String::new()
                }
            } else {
                cli::output::config_get(entry, json, style)?
            }
        }
        cli::ConfigAction::Set { key, value, target } => {
            let key = infra::config::key(&key)?;
            let path = config_target(current_dir, user_config, target)?;
            warning = inspection
                .origin(key)
                .and_then(|origin| override_warning(key, origin, target, &path, current_dir));
            let lock_dir = infra::host::config_lock_dir();
            let edit = infra::config::set(&path, key, &value, lock_dir.as_deref())?;
            append_warning(&mut warning, edit.warning);
            cli::output::config_set(key.as_str(), &edit.value, &path, json, style)?
        }
        cli::ConfigAction::Unset { key, target } => {
            let key = infra::config::key(&key)?;
            let path = config_target(current_dir, user_config, target)?;
            let lock_dir = infra::host::config_lock_dir();
            let edit = infra::config::unset(&path, key, lock_dir.as_deref())?;
            append_warning(&mut warning, edit.warning);
            cli::output::config_unset(key.as_str(), &path, edit.value, json, style)?
        }
        cli::ConfigAction::Path => {
            let path = user_config.ok_or(infra::config::ConfigCommandError::MissingUserPath)?;
            cli::output::config_path(path, json, style)?
        }
    };
    Ok(ConfigOutput {
        text,
        failed,
        warning,
    })
}

fn append_warning(warning: &mut Option<String>, added: Option<String>) {
    let Some(added) = added else {
        return;
    };
    if let Some(existing) = warning {
        existing.push_str("\nwarning: ");
        existing.push_str(&added);
    } else {
        *warning = Some(added);
    }
}

fn config_target(
    current_dir: &Path,
    user_config: Option<&Path>,
    target: cli::ConfigTargetArgs,
) -> Result<PathBuf, infra::config::ConfigCommandError> {
    if target.local {
        Ok(current_dir.join(".nib.local.toml"))
    } else if target.project {
        Ok(infra::config::project_config_path(current_dir)?)
    } else {
        user_config
            .map(Path::to_owned)
            .ok_or(infra::config::ConfigCommandError::MissingUserPath)
    }
}

fn override_warning(
    key: domain::ConfigKey,
    origin: &domain::ConfigOrigin,
    target: cli::ConfigTargetArgs,
    path: &Path,
    current_dir: &Path,
) -> Option<String> {
    let source = match origin {
        domain::ConfigOrigin::Environment(variable) => {
            format!("environment variable {variable}")
        }
        domain::ConfigOrigin::File(origin_path)
            if origin_path != path
                && ((!target.local && !target.project)
                    || (target.project && origin_path == &current_dir.join(".nib.local.toml"))) =>
        {
            origin_path.display().to_string()
        }
        domain::ConfigOrigin::Default | domain::ConfigOrigin::File(_) => return None,
    };
    Some(format!("{} is still overridden by {source}", key.as_str()))
}

pub fn run(
    cli: Cli,
    config: &Config,
    now: &Zoned,
    demo: bool,
    cache_directory: Option<&Path>,
    style: Style,
) -> miette::Result<String> {
    let json = cli.json;
    let action = cli
        .command
        .into_action(now, config)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("completion requests must use completion_output"))?;
    if demo {
        let backends = infra::demo::backends(now);
        app::commands::run(
            action,
            json,
            now,
            &backends,
            style,
            &mut infra::host::Memory::new(),
        )
    } else {
        let backends = infra::system_backends(infra::completions::CacheFile::new(
            cache_directory,
            now.timestamp().as_second(),
        ));
        app::commands::run(
            action,
            json,
            now,
            &backends,
            style,
            &mut infra::host::System,
        )
    }
}

pub fn completion_output(
    cli: &Cli,
    cache_directory: Option<&Path>,
    now: jiff::Timestamp,
    demo: bool,
) -> Option<String> {
    match cli.completion_request()? {
        cli::CompletionRequest::Script(shell) => Some(cli::completions::script(shell)),
        cli::CompletionRequest::Values { kind, refresh } => {
            Some(completion_values(kind, refresh, cache_directory, now, demo))
        }
    }
}

fn completion_values(
    kind: cli::CompletionKind,
    refresh: bool,
    cache_directory: Option<&Path>,
    now: jiff::Timestamp,
    demo: bool,
) -> String {
    if refresh {
        infra::completions::refresh(kind, cache_directory, now.as_second(), demo);
        String::new()
    } else {
        infra::completions::values(kind, cache_directory, now.as_second(), demo).join("\n")
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};

    use super::*;

    #[test]
    fn demo_log_uses_an_empty_in_memory_journal() {
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();
        let cli = Cli::try_parse_from(["nib", "log", "--json"]).unwrap();
        assert_eq!(
            run(cli, &Config::default(), &now, true, None, Style::default(),).unwrap(),
            "[]"
        );
    }
}

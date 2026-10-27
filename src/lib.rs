mod app;
mod cli;
mod domain;
mod infra;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jiff::Zoned;

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
    let error = inspection.error().map(ToString::to_string);
    let checks = app::doctor::checks(&probe, &config_files, error.as_deref());
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
            let edit = infra::config::set(&path, key, &value)?;
            append_warning(&mut warning, edit.warning);
            cli::output::config_set(key.as_str(), &edit.value, &path, json, style)?
        }
        cli::ConfigAction::Unset { key, target } => {
            let key = infra::config::key(&key)?;
            let path = config_target(current_dir, user_config, target)?;
            let edit = infra::config::unset(&path, key)?;
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
    style: Style,
) -> miette::Result<String> {
    let backends = if demo {
        infra::demo::backends(now)
    } else {
        infra::system_backends()
    };
    app::commands::run(cli, config, now, &backends, style, &mut infra::host::System)
}

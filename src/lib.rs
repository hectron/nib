mod app;
mod cli;
mod domain;
mod infra;

use std::ffi::OsString;
use std::path::Path;

use jiff::Zoned;

pub use cli::Cli;
pub use cli::output::Style;
pub use domain::Config;
pub use infra::config::{ConfigError, ConfigInspection};

pub struct DoctorOutput {
    pub text: String,
    pub failed: bool,
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

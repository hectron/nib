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
pub use infra::config::ConfigError;

pub fn load_config(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Config, ConfigError> {
    infra::config::load(start, user_config, env)
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

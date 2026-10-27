use std::io::{self, Write};

use clap::Parser;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use miette::IntoDiagnostic;

fn main() -> miette::Result<()> {
    let cli = nib::Cli::parse();
    let current_dir = std::env::current_dir().into_diagnostic()?;
    let user_config = user_config_path();
    let config = nib::load_config(&current_dir, user_config.as_deref(), |name| {
        std::env::var_os(name)
    })?;
    let now = Zoned::new(Timestamp::now(), local_zone());
    let demo = std::env::var_os("NIB_DEMO").is_some_and(|value| !value.is_empty());
    let out = nib::run(cli, &config, &now, demo, nib::Style::detect())?;
    if out.is_empty() {
        return Ok(());
    }
    match writeln!(io::stdout().lock(), "{out}") {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.into_diagnostic(),
    }
}

fn user_config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .map(|root| root.join("nib/config.toml"))
}

fn local_zone() -> TimeZone {
    std::env::var_os("TZ")
        .is_none()
        .then(read_localtime)
        .flatten()
        .unwrap_or_else(TimeZone::system)
}

fn read_localtime() -> Option<TimeZone> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let (_, name) = target.to_str()?.rsplit_once("zoneinfo/")?;
    TimeZone::tzif(name, &std::fs::read("/etc/localtime").ok()?).ok()
}

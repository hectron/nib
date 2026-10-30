use std::io::{self, Write};

use clap::Parser;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use miette::IntoDiagnostic;

fn main() -> miette::Result<()> {
    let cli = nib::Cli::parse();
    let demo = std::env::var_os("NIB_DEMO").is_some_and(|value| !value.is_empty());
    let now = Timestamp::now();
    let cache_directory = cache_directory();
    if let Some(out) = nib::completion_output(&cli, cache_directory.as_deref(), now, demo) {
        return print_output(&out);
    }
    let current_dir = std::env::current_dir().into_diagnostic()?;
    let user_config = user_config_path();
    let inspection = nib::inspect_config(&current_dir, user_config.as_deref(), |name| {
        std::env::var_os(name)
    });
    if cli.is_doctor() {
        let result = nib::doctor(
            &inspection,
            std::env::var_os("TERM_PROGRAM"),
            std::env::var_os("VISUAL"),
            std::env::var_os("EDITOR"),
            cli.json,
            nib::Style::detect(),
        )?;
        print_output(&result.text)?;
        if result.failed {
            std::process::exit(1);
        }
        return Ok(());
    }
    if cli.is_config() {
        let result = nib::run_config(
            cli,
            inspection,
            &current_dir,
            user_config.as_deref(),
            config_lock_directory().as_deref(),
            nib::Style::detect(),
        )?;
        print_output(&result.text)?;
        if let Some(warning) = result.warning {
            writeln!(io::stderr().lock(), "warning: {warning}").into_diagnostic()?;
        }
        if result.failed {
            std::process::exit(1);
        }
        return Ok(());
    }
    let config = inspection.into_config()?;
    let now = Zoned::new(now, local_zone());
    let out = nib::run(
        cli,
        &config,
        &now,
        demo,
        cache_directory.as_deref(),
        nib::Style::detect(),
    )?;
    print_output(&out)
}

fn print_output(out: &str) -> miette::Result<()> {
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

fn cache_directory() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .map(|home| home.join("Library/Caches/nib"))
}

fn config_lock_directory() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .map(|home| home.join("Library/Application Support/nib/backups"))
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

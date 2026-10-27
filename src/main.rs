mod backend;
mod cli;
mod commands;
mod demo;
mod edit;
mod host;
mod markdown;
mod output;

use std::io::{self, Write};

use clap::Parser;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use miette::IntoDiagnostic;

fn main() -> miette::Result<()> {
    let cli = cli::Cli::parse();
    let now = Zoned::new(Timestamp::now(), local_zone());
    let backends = if std::env::var_os("NIB_DEMO").is_some_and(|v| !v.is_empty()) {
        demo::backends(&now)
    } else {
        backend::Backends::system()
    };
    let out = commands::run(
        cli,
        &now,
        &backends,
        output::Style::detect(),
        &mut host::System,
    )?;
    if out.is_empty() {
        return Ok(());
    }
    match writeln!(io::stdout().lock(), "{out}") {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.into_diagnostic(),
    }
}

// TimeZone::system() scans every file in zoneinfo on first use (3-8 ms);
// reading the /etc/localtime link directly takes about 0.1 ms.
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

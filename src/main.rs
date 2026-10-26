mod backend;
mod cli;
mod commands;
mod output;

use std::io::{self, Write};

use clap::Parser;
use miette::IntoDiagnostic;

fn main() -> miette::Result<()> {
    let cli = cli::Cli::parse();
    let out = commands::run(cli, &backend::Backends::system())?;
    if out.is_empty() {
        return Ok(());
    }
    match writeln!(io::stdout().lock(), "{out}") {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.into_diagnostic(),
    }
}

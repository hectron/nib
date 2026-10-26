# AGENTS.md

## Overview and vision

`jot` is a high-performance, developer-centric command-line interface for Apple's ecosystem apps: **Apple Notes**, **Reminders**, **Calendar**, and eventually **Journal**.

It replaces clunky, slow legacy bridges with a clean, ergonomic, fast Rust binary for friction-free capture, querying, and task management from the terminal.

## Design philosophy

The interface, error handling, output formatting, and developer experience take after modern high-performance tools:

- **`uv`** and **`mise`** — speed, clean subcommands, and fast defaults.
- **`ripgrep`** and **`eza`** — crisp formatting, high readability, and Unix-style ergonomics.

## Technical stack

1. **Language:** Rust.
2. **Command-line framework:** `clap` 4 with the derive macros, git-style subcommands, and single-letter aliases.
3. **Diagnostics and output:** `miette` for user-facing errors and `comfy-table` for tables.
4. **Testing:** `insta` snapshots for command output, plus unit tests for data transformation.
5. **Apple integration:**
   - Objective-C runtime bridge (`objc2`, `core-foundation`) where native APIs allow it, such as `EventKit` for Reminders and Calendar.
   - Structured SQLite reads or `osascript` subprocesses for Apple Notes, which has no public API.

## Command structure

```bash
jot note new "Meeting with Team" --folder Work
jot task add "Review pull request" --due today --priority high
jot cal list --week

jot n "Quick thought capture"
jot t "Buy groceries"
```

## Code layout

- `src/cli.rs` — `clap` definitions. Every command line resolves to one `Action`. `n` and `t` take the rest of the line as the title; `note`, `task`, and `cal` take only subcommands.
- `src/commands.rs` — dispatches on `Action` and returns the text to print, so tests run commands against fake backends.
- `src/backend.rs` — domain types and one trait per app (`Notes`, `Reminders`, `Calendar`). `Backends::system()` picks the real implementations; all are stubs today.
- `src/output.rs` — table and JSON rendering. `main` decides color and width once (`Style::detect`), and tests pass a fixed `Style`. Text output strips control characters.

## Rules for changes

- Keep modules separated by concern: Apple wrappers, formatting, parsing, and configuration.
- Keep startup overhead under 50 ms. Initialize Apple frameworks lazily, on first use, never in `Backends::system()`.
- Every command that prints data must support `--json`.
- Don't give one command both a positional argument and subcommands. `clap` can't tell a title from a subcommand name, which is why `n` and `t` are separate commands.
- Never let table rendering detect the terminal on its own; take color and width from `Style`.
- Add or update `insta` snapshots for any change to command output, and review them before accepting.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` before committing.

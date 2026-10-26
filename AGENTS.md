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

- `src/cli.rs` — `clap` definitions and the bare-form rules (a bare title captures, no arguments lists).
- `src/commands.rs` — dispatch. Returns the text to print, so tests run commands against fake backends.
- `src/backend.rs` — domain types and one trait per app (`Notes`, `Reminders`, `Calendar`). `Backends::system()` picks the real implementations; all are stubs today.
- `src/output.rs` — table and JSON rendering.

## Rules for changes

- Keep modules separated by concern: Apple wrappers, formatting, parsing, and configuration.
- Keep startup overhead under 50 ms. Initialize Apple frameworks lazily, on first use, never in `Backends::system()`.
- Every command that prints data must support `--json`.
- Add or update `insta` snapshots for any change to command output, and review them before accepting.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` before committing.

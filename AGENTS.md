# AGENTS.md

## Overview and vision

`nib` is a high-performance, developer-centric command-line interface for Apple's ecosystem apps: **Apple Notes**, **Reminders**, **Calendar**, and eventually **Journal**.

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
6. **Dates and times:** `jiff`.

## Command structure

```bash
nib note new "Meeting with Team" --folder Work
nib task add "Review pull request" --due today --priority high
nib cal list --week

nib n "Quick thought capture"
nib t "Buy groceries"
```

## Code layout

- `src/lib.rs` — curated library facade. It exposes the CLI entry points while keeping layer internals private.
- `src/main.rs` — thin binary shell: parses arguments; reads the environment, current directory, clock, and local time zone once; calls the library; and prints its result.
- `src/domain/` — pure app types, configuration values, backend traits, and errors. Due dates are `jiff::civil::Date`; event starts are `Zoned`.
- `src/app/commands.rs` — dispatches on `Action` and returns the text to print, so tests run commands against fake backends.
- `src/app/markdown.rs` — renders reminder snapshots as Markdown, parses edited files, and plans validated reminder changes without touching Apple data.
- `src/app/edit.rs` — runs the `task edit` flow: snapshot, secure temporary file, editor, validation, preview, confirmation, backup, and apply.
- `src/infra/config.rs` — discovers and merges user, parent-directory, local, and environment configuration. Inputs are injected so tests never depend on the process environment.
- `src/infra/demo.rs` — sample notes, reminders, and events for `NIB_DEMO=1`, with dates relative to `now`. Nothing is saved.
- `src/infra/host.rs` — owns editor, prompt, message, temporary-file, and backup filesystem I/O.
- `src/infra/notes.rs` — invokes Apple Notes through `osascript`, passing data as arguments and parsing batched JXA list results.
- `src/infra/eventkit.rs` — the only unsafe module. It implements Reminders and Calendar with EventKit on macOS, converts callback data to owned Rust values in the callback, and keeps Objective-C calls behind small safe helpers.
- `src/cli/mod.rs` — `clap` definitions. Every command line resolves to one `Action`; relative input is resolved against the supplied clock and configuration.
- `src/cli/output.rs` — table and JSON rendering. Terminal style is detected once; text strips control characters; JSON uses private output types.

See [`STYLE.md`](STYLE.md) for the repository's concrete formatting, naming, architecture, error, unsafe, and lint rules.

## Rules for changes

- Keep modules separated by concern: Apple wrappers, formatting, parsing, and configuration.
- Keep startup overhead under 50 ms. Initialize Apple frameworks lazily, on first use, never in `infra::system_backends()`.
- Avoid `jiff::Zoned::now()`, `TimeZone::system()`, and `TimeZone::get()` on the startup path. jiff scans the whole zoneinfo directory on the first lookup by name, which costs 3–8 ms.
- Only `main` reads the clock. Code that needs the time takes `now` as a parameter, and tests pass a fixed time in a fixed-offset zone, or a POSIX TZ rule when they need daylight saving changes.
- Only `main` reads the real environment and current directory for configuration. Configuration loading takes those inputs explicitly and applies defaults while building an `Action`, so backends never discover configuration.
- Every command that prints data must support `--json`.
- Treat `--json` output as an interface that scripts depend on. Define its fields only in the JSON output types in `src/cli/output.rs`, never by deriving `Serialize` on a domain type. Every object starts with the item's `id`. Adding a field is fine; renaming or removing one breaks scripts, so do it only on purpose and say so in the commit. Every JSON shape has a snapshot in `src/app/commands.rs`.
- Don't give one command both a positional argument and subcommands. `clap` can't tell a title from a subcommand name, which is why `n` and `t` are separate commands.
- Never let table rendering detect the terminal on its own; take color and width from `Style`.
- Add or update `insta` snapshots for any change to command output, and review them before accepting.
- Never send an Objective-C object across threads. EventKit calls completion blocks on its own queue, so map results to Rust values inside the block and send those.
- The default test suite must never touch real Apple data. Tests that read it are `#[ignore]`d; tests that write also require `NIB_LIVE_WRITE=1` and delete what they create. CLI tests in `tests/cli.rs` run the real binary, so they may only run commands that hit a stub (`nib n` today) or set `NIB_DEMO=1`; `nib t` or `nib cal` there would ask macOS for access and hang.
- macOS grants EventKit access to the terminal app, not to `nib`, so no embedded Info.plist is needed. Error help should name the terminal (`terminal_name`).
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` before committing.

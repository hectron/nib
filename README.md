# nib

Fast command-line interface for Apple Notes, Reminders, and Calendar.

> **Status:** the command-line interface, output formatting, and tests are in place. No Apple app integration is wired up yet, so every command currently exits with a "not implemented" error.

```bash
nib note new "Meeting with Team" --folder Work
nib task add "Review pull request" --due today --priority high
nib cal list --week

# Quick capture: every word after n or t is the title
nib n Quick thought
nib t Buy groceries --due today
nib t                # no title lists reminders
```

| Command | Alias | With no subcommand or title |
| --- | --- | --- |
| `nib note` | | lists notes; `-f Work` filters by folder |
| `nib task` | `todo` | lists reminders |
| `nib cal` | `c` | lists today's events; `-w` lists the next seven days |
| `nib n [TITLE]...` | | lists notes |
| `nib t [TITLE]...` | | lists reminders |

`n` and `t` never read a word as a subcommand, so `nib n list` creates a note called "list". `note` and `task` take only subcommands, so use `nib note new "Title"` with the long form.

`--due` takes `today`, `tomorrow`, or a date such as `2026-11-03`.

Every command accepts `--json` for piping into other tools. Each JSON object starts with an `id` that identifies the note, reminder, or event, and dates use ISO 8601: `"2026-10-27"` for due dates and `"2026-10-26T10:00:00-04:00"` for event starts.

## Development

```bash
cargo test                      # unit, snapshot, and CLI tests
cargo clippy --all-targets -- -D warnings
cargo install --path .
```

Snapshot tests use [insta](https://insta.rs). After an intended output change, run `cargo insta review` (install with `cargo install cargo-insta`).

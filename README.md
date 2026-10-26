# jot

Fast command-line interface for Apple Notes, Reminders, and Calendar.

> **Status:** the command-line interface, output formatting, and tests are in place. No Apple app integration is wired up yet, so every command currently exits with a "not implemented" error.

```bash
jot note new "Meeting with Team" --folder Work
jot task add "Review pull request" --due today --priority high
jot cal list --week

# Quick capture: every word after n or t is the title
jot n Quick thought
jot t Buy groceries --due today
jot t                # no title lists reminders
```

| Command | Alias | With no subcommand or title |
| --- | --- | --- |
| `jot note` | | lists notes; `-f Work` filters by folder |
| `jot task` | `todo` | lists reminders |
| `jot cal` | `c` | lists today's events; `-w` lists the next seven days |
| `jot n [TITLE]...` | | lists notes |
| `jot t [TITLE]...` | | lists reminders |

`n` and `t` never read a word as a subcommand, so `jot n list` creates a note called "list". `note` and `task` take only subcommands, so use `jot note new "Title"` with the long form.

Every command accepts `--json` for piping into other tools.

## Development

```bash
cargo test                      # unit, snapshot, and CLI tests
cargo clippy --all-targets -- -D warnings
cargo install --path .
```

Snapshot tests use [insta](https://insta.rs). After an intended output change, run `cargo insta review` (install with `cargo install cargo-insta`).

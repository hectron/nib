# jot

Fast command-line interface for Apple Notes, Reminders, and Calendar.

> **Status:** the command-line interface, output formatting, and tests are in place. No Apple app integration is wired up yet, so every command currently exits with a "not implemented" error.

```bash
jot note new "Meeting with Team" --folder Work
jot task add "Review pull request" --due today --priority high
jot cal list --week

# Shorthand: a bare title captures, no arguments lists
jot n "Quick thought"
jot t "Buy groceries"
jot t
```

| Command | Aliases | Bare form |
| --- | --- | --- |
| `jot note` | `n` | `jot n "title"` creates, `jot n` lists |
| `jot task` | `t`, `todo` | `jot t "title"` adds, `jot t` lists |
| `jot cal` | `c` | `jot c` lists today, `jot c --week` lists seven days |

Every command accepts `--json` for piping into other tools.

## Development

```bash
cargo test                      # unit, snapshot, and CLI tests
cargo clippy --all-targets -- -D warnings
cargo install --path .
```

Snapshot tests use [insta](https://insta.rs). After an intended output change, run `cargo insta review` (install with `cargo install cargo-insta`).

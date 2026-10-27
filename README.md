# nib

Fast command-line interface for Apple Notes, Reminders, and Calendar.

Requires macOS 26 or later.

> **Status:** Reminders and Calendar work: `nib t` lists incomplete reminders, `nib t <title>` adds one, and `nib cal` lists events. Notes still exits with a "not implemented" error.
>
> To try Notes anyway, set `NIB_DEMO=1` and every command runs against built-in sample data, with dates relative to today. Nothing is saved.
>
> ```bash
> NIB_DEMO=1 nib n
> NIB_DEMO=1 nib n --json
> ```

## Permissions

The first Reminders command and the first Calendar command each ask macOS for access. macOS grants it to your terminal app (Terminal, iTerm, WezTerm, and so on), not to `nib`, so it shows up under that app in System Settings > Privacy & Security > Reminders and > Calendars. Calendar needs full access; "add only" can't list events. Each terminal app asks once. The VS Code terminal, ssh sessions, and cron jobs can't show the prompt; grant access from a regular terminal first.

## Usage

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

## Editing reminders

`nib task edit` opens incomplete reminders as Markdown in `$VISUAL` or `$EDITOR`. Edit titles and notes, add due dates and priorities, check boxes to complete reminders, add or delete task lines, or move a task under another list heading.

```markdown
## Work 🆔 a1b2c3d4

- [ ] Review pull request 🔼 📅 2026-11-03 🆔 e5f6a7b8
    Check the release notes.
```

Save and quit to preview the changes. Nothing is applied without confirmation. Before updating or deleting existing reminders, `nib` saves a JSON backup in `~/Library/Application Support/nib/backups`.

Subtasks, tags, flags, sections, and due times are not shown.

Every command accepts `--json` for piping into other tools. Each JSON object starts with an `id` that identifies the note, reminder, or event, and dates use ISO 8601: `"2026-10-27"` for due dates and `"2026-10-26T10:00:00-04:00"` for event starts. Events also have `all_day`. Every occurrence of a recurring event shares Apple's identifier, so an event's `id` adds the occurrence's start in UTC, such as `"ABC123@2026-10-27T14:00:00Z"`.

## Configuration

Settings come from command-line flags, environment variables, directory configuration, the user configuration, and built-in defaults, in that order. Directory configuration uses `.nib.local.toml`, then `nib.toml` or `.nib.toml`, in the current directory and each parent; nearer directories override farther ones, field by field. The user configuration is `$XDG_CONFIG_HOME/nib/config.toml`, or `~/.config/nib/config.toml` when `XDG_CONFIG_HOME` is unset.

| Key | Environment variable | Command-line flag |
| --- | --- | --- |
| `task.list` | `NIB_TASK_LIST` | `-l, --list NAME` on `nib t` and `nib task add`; also defaults `nib task edit --list` |
| `task.priority` | `NIB_TASK_PRIORITY` | `-p, --priority` on `nib t` and `nib task add` |
| `cal.calendars` | `NIB_CAL_CALENDARS`, comma-separated | repeatable `--calendar NAME` on `nib cal` |
| `note.folder` | `NIB_NOTE_FOLDER` | `-f, --folder` on `nib n` and `nib note` |

```toml
[task]
list = "Snowglobe"
priority = "medium"

[cal]
calendars = ["Work"]
```

## Development

```bash
cargo test                      # unit, snapshot, and CLI tests
cargo clippy --all-targets -- -D warnings
cargo install --path .
```

Snapshot tests use [insta](https://insta.rs). After an intended output change, run `cargo insta review` (install with `cargo install cargo-insta`).

Tests that touch your real reminders and calendars are skipped by default. Run them from a terminal that already has access:

```bash
cargo test -- --ignored lists_real                                   # read only: reminders and events
NIB_LIVE_WRITE=1 cargo test -- --ignored adds_and_removes_a_real_reminder  # adds one, then deletes it
```

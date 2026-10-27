# Configuration

Put shared defaults in `nib.toml`:

```toml
[task]
list = "Work"
priority = "medium"
parse_inline = true

[cal]
calendars = ["Work", "Birthdays"]

[note]
folder = "Inbox"
```

Now capture with fewer flags:

```bash
nib t "Review pull request"
nib task edit
nib cal --week
```

## Files

The user configuration is:

- `$XDG_CONFIG_HOME/nib/config.toml` when `XDG_CONFIG_HOME` is set and nonempty.
- `~/.config/nib/config.toml` otherwise.

For directory-specific defaults, nib walks from the filesystem root to the current directory. In each directory it reads `nib.toml`, or `.nib.toml` when `nib.toml` is absent, followed by `.nib.local.toml`.

Use `nib.toml` for settings you commit. Use `.nib.local.toml` for personal overrides.

## Precedence

Settings merge field by field. From lowest to highest precedence:

1. Built-in defaults.
2. User configuration.
3. Directory configuration, from the farthest ancestor to the current directory.
4. Environment variables.
5. Command-line flags.

A nearer file only replaces the fields it defines.

## Settings

| TOML key | Environment variable | Values | Used by |
| --- | --- | --- | --- |
| `task.list` | `NIB_TASK_LIST` | List name | `nib t`, `nib task add`, and `nib task edit` |
| `task.priority` | `NIB_TASK_PRIORITY` | `low`, `medium`, or `high` | `nib t` and `nib task add` |
| `task.parse_inline` | `NIB_TASK_PARSE_INLINE` | `true` or `false` | Trailing due dates, priorities, and lists for `nib t` and `nib task add` |
| `cal.calendars` | `NIB_CAL_CALENDARS` | Array in TOML; comma-separated names in the environment | `nib cal` |
| `note.folder` | `NIB_NOTE_FOLDER` | Folder name | `nib n` and `nib note new` |

Calendar names from `NIB_CAL_CALENDARS` are trimmed, and empty names are ignored.

Inline task parsing is on by default. Set `task.parse_inline = false` or `NIB_TASK_PARSE_INLINE=false` to keep trailing tokens as part of reminder titles. Explicit `--due`, `--priority`, and `--list` flags still work. See [Quick-add reminders](quick-add.md) for the recognized tokens.

## Override a default

```bash
nib t "File expenses" --list Personal --priority low
nib task edit --list Personal
nib cal --calendar Work --calendar Birthdays
nib note list --folder Archive
```

`-l`, `-p`, and `-f` are the short forms of `--list`, `--priority`, and `--folder`. `-w` is the short form of `--week`.

The configured note folder is a creation default. Use `--folder` explicitly when listing notes.

Unknown TOML keys and invalid environment values are errors, so misspelled settings do not fail silently.

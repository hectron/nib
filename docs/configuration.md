# Configuration

Inspect the effective configuration and where each value came from:

```bash
nib config list
nib config list --origin
nib config get task.list
```

Change the user configuration without opening it in an editor:

```bash
nib config set task.list Work
nib config set task.priority high
nib config set cal.calendars 'Work, Personal'
nib config set tui.preset default
nib config unset task.priority
nib config path
```

`set` and `unset` edit the user configuration by default. Pass `--local` to edit `.nib.local.toml` in the current directory. `--project` edits `nib.toml` when it exists, otherwise `.nib.toml` when that exists, and otherwise creates `nib.toml`. Existing comments, line endings, and string quote style are preserved when the new value can use the same TOML quoting. Calendar values accept comma-separated names or a TOML array such as `["Work", "Personal"]`; string values such as list and folder names are passed directly.

Like Git, `nib config set` and `nib config unset` replace the configuration file atomically. Replacing a read-only file is allowed when its directory is writable, and replacing one name of a hard-linked file splits that name from the other links.

While `set` or `unset` edits a file, it holds a lock on that file, so a second `set` or `unset` on the same file fails with `nib::config::locked` instead of overwriting the first change. The lock is a `flock` on a file in `~/Library/Application Support/nib/backups` named `config-<hash>.lock`, where the hash comes from the configuration file's resolved path. macOS releases the lock when nib exits, even after a crash, so it never has to be removed by hand.

The lock is per user, because it lives in your home folder, so two people editing a shared `nib.toml` don't block each other. Each write replaces the file atomically, so no one sees a half-written file, but each edit rewrites the whole file from the copy it read. If another user changes a different key at the same time, one of the two changes can be lost. A lock folder that can't be created or opened is reported with `nib::config::lock` and the folder or lock file's path, and so is a `set` or `unset` run without `HOME`.

nib 0.4.1 and earlier locked by creating `<file>.lock` next to the configuration file, such as `nib.toml.lock`. nib no longer creates or reads those files, so any that remain can be deleted. `nib doctor` reports them as a warning.

All config commands that print output accept `--json`. In text mode, `get` exits with status 1 and prints nothing when a setting is unset and has no built-in default. In JSON mode it still exits with status 1 but prints an object whose `value` is `null`. JSON list entries always include a typed `origin` object; `--origin` controls only the text table. Unsetting a key absent from the selected file succeeds without creating or rewriting anything and reports that nothing changed.

Add a `nib.toml` to a work repository:

```toml
[task]
list = "Work"

[cal]
calendars = ["Work"]
show_calendar = true

[tui]
preset = "default"
```

These defaults apply in that directory and every directory below it.

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

## Editor schema

The published [JSON schema](config.schema.json) provides completion and validation. Taplo recognizes this comment at the top of any nib configuration file:

```toml
#:schema https://raw.githubusercontent.com/hectron/nib/main/docs/config.schema.json
```

## Precedence

Settings merge field by field. From lowest to highest precedence:

1. Built-in defaults.
2. User configuration.
3. Directory configuration, from the farthest ancestor to the current directory.
4. Environment variables.
5. Command-line flags.

A nearer file only replaces the fields it defines.

Keymap state tables merge chord by chord. Set a chord to `"none"` to remove a binding inherited from the preset or a lower-precedence file.

## Settings

| TOML key | Environment variable | Values | Used by |
| --- | --- | --- | --- |
| `task.list` | `NIB_TASK_LIST` | List name | `nib t`, `nib task add`, and `nib task edit` |
| `task.priority` | `NIB_TASK_PRIORITY` | `low`, `medium`, or `high` | `nib t` and `nib task add` |
| `task.parse_inline` | `NIB_TASK_PARSE_INLINE` | `true` or `false` | Trailing due dates, priorities, and lists for `nib t` and `nib task add` |
| `cal.calendars` | `NIB_CAL_CALENDARS` | Array in TOML; comma-separated names in the environment | `nib cal` |
| `cal.show_calendar` | `NIB_CAL_SHOW_CALENDAR` | `true` or `false` | Calendar column in `nib cal` and `nib today` event tables |
| `note.folder` | `NIB_NOTE_FOLDER` | Folder name | `nib n` and `nib note new` |
| `tui.preset` | `NIB_TUI_PRESET` | `default` | Interactive key bindings |

Calendar names from `NIB_CAL_CALENDARS` are trimmed, and empty names are ignored.

The event tables in `nib cal` and `nib today` show a `CALENDAR` column only when `cal.show_calendar = true`, `NIB_CAL_SHOW_CALENDAR=true`, or `--show-calendar` is given. The flag only turns the column on; run `nib config unset cal.show_calendar` or set it to `false` to turn it off. JSON event objects always include `calendar`.

Inline task parsing is on by default. Set `task.parse_inline = false` or `NIB_TASK_PARSE_INLINE=false` to keep trailing tokens as part of reminder titles. Explicit `--due`, `--priority`, and `--list` flags still work. See [Quick-add reminders](quick-add.md) for the recognized tokens.

Bindings live under `[tui.keys.browse]`, `[tui.keys.filter]`, and `[tui.keys.capture]`. They accept one action string or an array of actions. See [Keymaps](keymap.md) for chord syntax, actions, defaults, and unbinding.

## Override a default

```bash
nib t "File expenses" --list Personal --priority low
nib task edit --list Personal
nib cal --calendar Work --calendar Birthdays
nib note list --folder Archive
```

`-l`, `-p`, and `-f` are the short forms of `--list`, `--priority`, and `--folder`. `-w` is the short form of `--week`.

The configured note folder is a creation default. Use `--folder` explicitly when listing notes.

Unknown TOML keys and invalid file or environment values are errors, so misspelled settings do not fail silently. File diagnostics highlight the exact source span.

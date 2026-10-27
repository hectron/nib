# nib

Capture and query Apple Notes, Reminders, and Calendar without leaving the terminal.

![A terminal demo of nib](docs/demo.gif)

Built for macOS 26 and later. Reminders and Calendar use live Apple data. Apple Notes integration is not implemented yet, but its commands work with the demo backend.

## Install

Install the latest release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/hectron/nib/releases/latest/download/nib-installer.sh | sh
```

Or build the current version with Cargo:

```bash
cargo install --git https://github.com/hectron/nib
```

## Quick start

```bash
nib t "Buy groceries" --due today
nib t
nib task edit
nib cal --week
NIB_DEMO=1 nib n "Quick thought"
```

`n` and `t` are capture commands. With a title, they create an item; without one, they list items.

```bash
nib t "Review pull request" --due tomorrow --priority high
nib task add "Pay rent" --list Personal
nib cal --calendar Work
nib note list --folder Work
```

## Why nib

- Fast, focused commands with short aliases and useful defaults.
- Native EventKit access for Reminders and Calendar.
- Safe bulk reminder editing in `$VISUAL` or `$EDITOR`.
- Pretty terminal tables and script-friendly JSON.
- Layered TOML configuration for personal and project defaults.
- A non-persistent demo backend with dates relative to today.

Try every command without touching Apple data:

```bash
NIB_DEMO=1 nib t
NIB_DEMO=1 nib cal --week
NIB_DEMO=1 nib note list
```

Nothing is saved while `NIB_DEMO=1` is set.

## Guides

- [Configuration](docs/configuration.md)
- [Editing reminders as Markdown](docs/task-edit.md)
- [macOS permissions](docs/permissions.md)
- [JSON output](docs/json.md)
- [Contributing](CONTRIBUTING.md)

Run `nib --help` or `nib <command> --help` for the complete command reference.

## License

[MIT](LICENSE)

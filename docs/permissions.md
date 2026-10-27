# macOS permissions

Run a Reminders or Calendar command from your regular terminal:

```bash
nib t
nib cal
```

The first command for each app asks macOS for Full Access. Reminders and Calendar are authorized separately. Calendar's Add Only permission is not enough because nib lists events.

macOS grants access to the terminal app that launches nib — Terminal, iTerm, Ghostty, WezTerm, or another host — not to the `nib` binary.

## Grant access manually

Open System Settings, then enable your terminal under:

```text
Privacy & Security > Reminders
Privacy & Security > Calendars
```

Run the command again after changing the setting.

Each terminal app has its own permission. Granting access to Terminal does not grant it to iTerm or an IDE terminal.

## Sessions without a prompt

The VS Code terminal, SSH sessions, and cron jobs cannot display the macOS permission prompt. Grant access once from a regular terminal app first, then retry from the noninteractive session.

If access is denied, nib names the terminal app when it can and prints the matching System Settings path.

Apple Notes integration is not implemented yet, so note commands do not request access outside the demo backend.

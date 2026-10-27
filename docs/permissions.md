# macOS permissions

Run a command for each app from your regular terminal:

```bash
nib note list
nib t
nib cal
```

The first Reminders and Calendar command asks macOS for Full Access. They are authorized separately. Calendar's Add Only permission is not enough because nib lists events.

The first Notes command asks to let your terminal control Apple Notes through macOS Automation. nib can capture notes and list their metadata. Locked notes appear in listings, but nib never reads their contents.

macOS grants access to the terminal app that launches nib — Terminal, iTerm, Ghostty, WezTerm, or another host — not to the `nib` binary.

## Grant access manually

Open System Settings, then enable your terminal under:

```text
Privacy & Security > Automation > your terminal > Notes
Privacy & Security > Reminders
Privacy & Security > Calendars
```

Run the command again after changing the setting.

Each terminal app has its own permission. Granting access to Terminal does not grant it to iTerm or an IDE terminal.

## Sessions without a prompt

The VS Code terminal, SSH sessions, and cron jobs cannot display the macOS permission prompt. Grant access once from a regular terminal app first, then retry from the noninteractive session.

If access is denied, nib names the terminal app when it can and prints the matching System Settings path. For Notes, allow the terminal to control Notes under **Privacy & Security > Automation**.

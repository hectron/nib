# macOS permissions

Check every permission at once without triggering a prompt:

```bash
nib doctor
```

`nib doctor` also checks your macOS version, configuration files, active keymap, and editor. Warnings don't fail the command; a failed check exits with status 1. Add `--json` for scripts.

To grant access, run a command for each app from your regular terminal:

```bash
nib note list
nib t
nib cal
```

The first Reminders and Calendar command asks macOS for Full Access. They are authorized separately. Calendar's Add Only permission is not enough because nib lists events.

The first Notes command asks to let your terminal control Apple Notes through macOS Automation. nib can capture notes and list their metadata. Locked notes appear in listings, but nib never reads their contents.

macOS grants access to the terminal app that launches nib — Terminal, iTerm, Ghostty, WezTerm, or another host — not to the `nib` binary.

Shell completion never asks for access. After you first use completions, nib stores reminder list, calendar, and Notes folder names in plain text at `~/Library/Caches/nib/completions.json` with mode `0600`. Reminder lists and calendars may be refreshed in the background when the cache is more than five minutes old, but only when the terminal already has Full Access. Notes folders are never queried during completion. The cache remains after access is revoked; delete the file to clear those names. `NIB_DEMO=1` never reads or writes the cache.

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

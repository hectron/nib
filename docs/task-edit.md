# Edit reminders as Markdown

Open every incomplete reminder in your editor:

```bash
nib task edit
```

Limit the file to one list:

```bash
nib task edit --list Work
```

nib uses `$VISUAL`, then `$EDITOR`, and falls back to `vi`. GUI editors must wait for the file to close; for example, set `EDITOR="code --wait"`.

## Format

```markdown
## Work 🆔 a1b2c3d4

- [ ] Review pull request ⏫ 📅 2026-11-03 09:00 🆔 e5f6a7b8
    Check the release notes.
- [ ] Draft announcement 🔼
```

- `##` starts an existing reminder list. Keep its `🆔` value.
- `- [ ]` is an incomplete reminder. Change it to `- [x]` to complete it.
- `⏫`, `🔼`, and `🔽` mean high, medium, and low priority.
- `📅 YYYY-MM-DD` is the due date. Add a 24-hour time as `📅 YYYY-MM-DD HH:MM`.
- A time in another zone includes its IANA name, such as `📅 2026-11-03 09:00 America/New_York`. Local, floating, and multi-token system zone names render without a zone. A time whose zone matches your local offset on its old date is treated as local: when a date or time edit changes it, nib re-pins it to your local zone at the new date and time, so the time you see stays the same. Floating and multi-token system zones keep their hidden zone when you change the date or time.
- Indent notes with four spaces or a tab beneath their reminder.
- A reminder without an `🆔` is new.
- HTML comments are ignored.

Edit a title or note in place. Remove the priority or due-date marker to clear that field. Delete an existing reminder line to delete it. Move a line beneath another list heading to move the reminder.

Due edits are component-wise: changing only the date keeps the time and zone, changing only the time keeps the date and zone, and removing the time keeps the date. The one exception is a hidden local zone, which re-pins as described above. An untouched marker is never written. Removing a visible foreign-zone suffix pins that time to the local zone. Adding a time to a date-only reminder also uses the local zone; when the local zone has no IANA name, nib pins it to the fixed offset in effect at the due time and shows that offset in the preview. If a due-date or due-time edit leaves an absolute alarm at its old instant, the preview warns `has an alarm that stays at …` in local time; relative alarms move with the due value and need no warning.

nib cannot create or rename lists in this file. Changes that touch a read-only list are rejected.

## Before anything changes

Saving the file only builds a plan. nib validates the Markdown, prints every add, update, completion, move, and deletion, then asks for confirmation.

Deleting more than three reminders requires typing the exact deletion count. An empty response never confirms a normal change.

Before updating, completing, moving, or deleting an existing reminder, nib writes its original data to:

```text
~/Library/Application Support/nib/backups
```

Backup names start with a local timestamp, include `-tasks-`, and end with a random suffix plus `.json` to avoid overwriting another backup. New reminders do not need a backup. nib warns when EventKit returned only enough data for a partial backup.

## Undo history

Every applied edit also writes a journal entry in the backups folder. List the history or undo the latest active edit with:

```bash
nib log
nib undo
```

Pass an entry from `nib log` to choose an older edit. Plain `nib undo` skips entries that are themselves undos; pass an undo entry's ID explicitly to reverse it.

Undoing a deletion re-creates the reminder with a new ID. It may lose alarms, repeats, or other metadata that EventKit does not expose. Keep the journal files in the backups folder: removing one changes the history nib can undo.

If an undo is interrupted, `nib log` shows its status as `pending` and later undo commands stop rather than risk applying it twice. Inspect the listed reminders, then run:

```bash
nib undo --resolve <ID>
```

nib lists what Reminders shows if the undo was applied, such as `"Oat milk" is re-created`. Type `applied` if Reminders matches, and nib marks the undo applied. Type `absent` only if none of it happened; nib then removes the pending record so the edit can be undone again. Any other answer, including pressing Return, changes nothing.

## Safety rules

- The edit file is created with owner-only permissions.
- Reminders with unsafe control characters in the title or notes, or line breaks in the title, are skipped and cannot be changed through the file.
- A file with no reminder lines is rejected; delete reminders in smaller groups.
- List and reminder IDs must remain unique and must come from the opened file.
- If a reminder changed in Apple Reminders while the editor was open, nib aborts instead of overwriting it.
- Parse errors can be reopened in the editor. If you stop or decline the preview, the edited file is kept and its path is printed.
- A failed apply keeps the edited file and leaves the EventKit batch uncommitted.

The Markdown format represents titles, notes, due dates and times, priorities, completion, and list placement. Alarms, repeats, and links are not rendered. Ordinary edits preserve them; a deletion loses them, and nib shows that loss in the preview. Removing an existing reminder's `🆔` turns it into a delete plus an add, which also loses that metadata.

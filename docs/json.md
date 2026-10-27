# JSON output

Add `--json` to a create or list command:

```bash
nib t --json
nib cal --week --json
nib note list --folder Work --json
```

Lists return arrays. Creating a note or reminder returns one object. Empty lists return `[]`, and optional values stay present as `null`.

## Notes

```json
{
  "id": "note-4",
  "title": "Quick thought",
  "folder": "Inbox"
}
```

Note objects contain `id`, `title`, and `folder`.

## Reminders

```json
{
  "id": "reminder-3",
  "title": "Review pull request",
  "due": "2026-11-03",
  "priority": "high"
}
```

Reminder objects contain `id`, `title`, `due`, and `priority`. Due dates use `YYYY-MM-DD`. Priority is `low`, `medium`, `high`, or `null`.

## Calendar events

```json
{
  "id": "ABC123@2026-10-27T14:00:00Z",
  "title": "Design review",
  "start": "2026-10-27T10:00:00-04:00",
  "all_day": false
}
```

Event objects contain `id`, `title`, `start`, and `all_day`. Starts use RFC 3339 with the local UTC offset.

Apple can reuse one identifier for every occurrence of a recurring event. nib appends the occurrence's UTC start to make each event ID distinct.

## Stability

JSON is pretty-printed. Every object starts with `id`, and field names are treated as a scripting interface. New fields may be added, but existing fields are not renamed or removed casually.

`nib task edit` remains an interactive Markdown workflow; `--json` does not replace its preview or confirmation.

# JSON output

Add `--json` to a create or list command:

```bash
nib t --json
nib today --json
nib cal --week --json
nib note list --folder Work --json
```

Lists return arrays. Creating a note or reminder returns one item object, while `nib today` returns an agenda object. Empty lists return `[]`, and optional values stay present as `null`.

## Notes

```json
{
  "id": "note-4",
  "title": "Quick thought",
  "folder": "Inbox",
  "modified": "2026-10-26T14:00:00Z",
  "locked": false
}
```

Note objects contain `id`, `title`, `folder`, `modified`, and `locked`. `modified` is an RFC 3339 timestamp or `null`; `locked` is a boolean. Locked notes are listed without reading their contents.

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

## Today's agenda

```json
{
  "overdue": [
    {
      "id": "reminder-1",
      "title": "Pay rent",
      "due": "2026-10-25",
      "priority": "high"
    }
  ],
  "due_today": [],
  "events": [
    {
      "id": "event-2",
      "title": "Standup",
      "start": "2026-10-26T09:30:00-04:00",
      "all_day": false
    }
  ]
}
```

`nib today --json` returns one object with `overdue`, `due_today`, and `events` arrays. The first two contain reminder objects, and `events` contains calendar event objects.

## Stability

JSON is pretty-printed. Every item object starts with `id`, and field names are treated as a scripting interface. New fields may be added, but existing fields are not renamed or removed casually.

`nib task edit` remains an interactive Markdown workflow; `--json` does not replace its preview or confirmation.

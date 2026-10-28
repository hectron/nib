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
  "due_time": "09:00",
  "due_time_zone": "America/New_York",
  "priority": "high"
}
```

Reminder objects contain `id`, `title`, `due`, `due_time`, `due_time_zone`, and `priority`. `due` keeps its `YYYY-MM-DD` shape. `due_time` uses `HH:MM`, or is `null` for a date-only or undated reminder. `due_time_zone` is an IANA name for a zoned time and is `null` for a floating time or a reminder with no time. Priority is `low`, `medium`, `high`, or `null`.

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

## Folders, lists, and calendars

`nib note folders --json` returns folder objects with `id`, `name`, `account_id`, and `account`. `nib task lists --json` and `nib cal calendars --json` return objects with `id`, `name`, `source_id`, `source`, `type`, `writable`, and `default`.

The account or source name distinguishes duplicate resource names for people, while `account_id` or `source_id` is the stable owner selector for scripts. Owner fields may be `null` if the bridge does not report them. EventKit resource types are `local`, `caldav`, `exchange`, `subscription`, `birthday`, or `unknown`.

Resource create and rename commands return the same single-object shape as their corresponding discovery command. For example, `nib --json task lists create Errands --source iCloud` returns the new reminder-list object.

## Today's agenda

```json
{
  "overdue": [
    {
      "id": "reminder-1",
      "title": "Pay rent",
      "due": "2026-10-25",
      "due_time": null,
      "due_time_zone": null,
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

## Doctor

`nib doctor --json` returns one object with `id`, `ok`, and `checks`. Each check contains `id`, `status` (`pass`, `warning`, or `fail`), `message`, and `fix`, which is `null` when nothing needs fixing. `ok` is `false` when any check fails.

## Stability

JSON is pretty-printed. Every item object starts with `id`, and field names are treated as a scripting interface. New fields may be added, but existing fields are not renamed or removed casually.

`nib task edit` remains an interactive Markdown workflow; `--json` does not replace its preview or confirmation.

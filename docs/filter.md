# Filtering reminders

Use `--filter` with `nib task list`, or with the default list form:

```bash
nib task list --filter "review pri:high"
nib task --filter "list:work !due:none"
nib task list -f "due:week !pri:low" --json
```

The expression is split into whitespace-separated tokens. Every token must match. Prefix a token with `!` to negate it.

| Field | Values |
| --- | --- |
| `list:` | Case-insensitive substring of the reminder list name |
| `cal:` | Case-insensitive substring of an event calendar name |
| `due:` | `today`, `overdue`, `week`, `none`, or `YYYY-MM-DD` |
| `pri:` | `high`, `medium`, `low`, or `none` |
| `is:` | `overdue`, `done`, `open`, or `event` |

The engine also supports event rows, but `nib task list` currently supplies reminders only, so `cal:` and `is:event` do not match anything in this command.

`due:week` includes overdue reminders, today, and the next seven days. Completed items are hidden unless the expression contains `is:done`.

Text without a recognized field searches the title and list or calendar name, ignoring case. It matches either a contiguous substring or characters in order, so `rvw` matches `Review`. A colon does not make an unknown prefix an error: `http://example.com` is ordinary search text.

Filters use AND only. There are no parentheses, OR expressions, regular expressions, ranking, or scoring.

# Change reminders in one step

Complete, reopen, prioritize, or move reminders without opening the whole `nib task edit` file:

```bash
nib task done "oat milk"
nib task priority high report
nib task priority none report
nib task move Work "pull request" groceries
```

| Command | Change |
| --- | --- |
| `nib task done <SELECTOR>...` | Marks open reminders completed. |
| `nib task reopen <SELECTOR>...` | Marks completed reminders open again. |
| `nib task priority <high\|medium\|low\|none> <SELECTOR>...` | Sets the priority, or clears it with `none`. |
| `nib task move <LIST> <SELECTOR>...` | Moves reminders to an existing, writable list. |

Each command prints one line per reminder it changed:

```text
Completed "Buy oat milk"
Moved "Review pull request" to Work
```

A reminder that already has the requested priority or is already in the target list is left alone. If nothing needs to change, nib prints `No changes.`

## Selectors

A selector is a reminder ID, as printed by `--json`, or part of a title. Title matching ignores case and must match exactly one reminder. `done`, `priority`, and `move` match titles among open reminders, while `reopen` matches completed ones. An ID selects its reminder in any list, but `done` refuses one that is already completed, and `reopen` refuses one that is still open.

Add `--list <NAME>` to match titles in one list only:

```bash
nib task done eggs --list Groceries
```

When a title matches no reminder, or more than one, nothing changes. An ambiguous selector lists every candidate with its ID and list:

```text
× "buy" matches 2 open reminders:
│   rem-1  "Buy oat milk"  (Home)
│   rem-2  "Buy eggs"  (Work)
```

nib resolves every selector before it changes anything, so one bad selector leaves all of the reminders as they were.

With live Reminders data, `nib task reopen` finds completed reminders by ID only; searching completed reminders by title isn't available yet. The `id` field in the output of `nib task done --json` is the ID to pass. Title search for `reopen` works with `NIB_DEMO=1`.

## Undo

These commands don't ask for confirmation. Each one writes the same backup and journal entry as `nib task edit`, so `nib log` shows it as `task_done`, `task_reopen`, `task_priority`, or `task_move`, and `nib undo` reverses it:

```bash
nib task done "oat milk"
nib undo
```

See [Undo history](task-edit.md#undo-history) for how the journal works.

## JSON

Add `--json` to print an array with one object per changed reminder. See [JSON output](json.md#reminder-changes) for its fields.

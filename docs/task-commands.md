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

`move` looks up `<LIST>` by its exact name first and falls back to a list ID, as printed by `nib task lists --json`.

Each command prints one line per reminder it changed:

```text
Completed "Buy oat milk"
Moved "Review pull request" to Work
```

A reminder that already has the requested priority or is already in the target list is left alone. If nothing needs to change, nib prints `No changes.`

On a repeating reminder, `done` completes the current occurrence the way Reminders does, and nib reports the state EventKit wrote back. This hasn't been checked against real repeating reminders yet.

## Selectors

A selector is a reminder ID, as printed by `--json`, or part of a title. Title matching ignores case and must match exactly one reminder. `done`, `priority`, and `move` match titles among open reminders, while `reopen` finds completed reminders by ID or title. An ID selects its reminder in any list, but `done` refuses one that is already completed, and `reopen` refuses one that is still open.

Add `--list <NAME>` to match titles in one list only. It takes an exact list name, not a list ID, and doesn't restrict selectors that are IDs:

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

`reopen` reads every completed reminder before it matches, so it can be slow with a very large completed history.

## Undo

These commands don't ask for confirmation. Before changing anything, each one writes the reminders' original data to `~/Library/Application Support/nib/backups`, the same folder `nib task edit` uses. It also writes a journal entry there, so `nib log` shows it as `task_done`, `task_reopen`, `task_priority`, or `task_move`, and `nib undo` reverses it. Deletes made from [`nib pick`](pick.md) or [`nib board`](board.md) appear as `task_delete`:

```bash
nib task done "oat milk"
nib undo
```

nib doesn't store a reminder's original completion date, so undoing a `reopen` marks the reminder completed again without restoring it. Reminders is expected to show the time of the undo as the completion date; this hasn't been checked against real data.

See [Undo history](task-edit.md#undo-history) for how the journal works.

## JSON

Add `--json` to print an array with one object per changed reminder. See [JSON output](json.md#reminder-changes) for its fields.

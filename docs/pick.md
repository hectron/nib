# Pick reminders interactively

`nib pick` opens an fzf-style prompt under your shell prompt. Type to filter, press Enter, and nib prints the chosen reminder IDs to standard output, one per line:

```bash
nib pick
nib pick --filter "list:work due:week"
nib task done $(nib pick -f "pri:high")
```

The prompt is drawn on the terminal itself, not on standard output, so it still works inside `$(...)` or with output redirected to a file. Standard input must be a terminal; otherwise `nib pick` exits with an error. When the prompt closes, nib clears it.

## Filtering

The query uses the same syntax as `nib task list --filter`: fuzzy title text plus `list:`, `due:`, `pri:`, and `is:` fields and `!` negation. See [Filtering reminders](filter.md). An invalid field value is shown under the prompt, and the results stay as they were until the query is valid again. While the query is invalid, Enter and `ctrl-o` do nothing. Completed reminders are loaded the first time the query contains `is:done`.

The highlighted reminder's list, due date, priority, link, and notes appear in a preview on the right when the terminal is at least 80 columns wide.

## Keys

| Key | Action |
| --- | --- |
| `enter` | Print the selected IDs, or the highlighted ID when nothing is selected, and exit |
| `tab` / `shift-tab` | Select or unselect the highlighted reminder and move down or up |
| `ctrl-n` / `down`, `ctrl-p` / `up` | Move the highlight |
| `ctrl-o` | Open the action menu |
| `esc`, `ctrl-g`, `ctrl-c` | Exit without printing anything (exit status 130) |
| `ctrl-a` / `home`, `ctrl-e` / `end` | Move to the start or end of the query |
| `ctrl-b` / `left`, `ctrl-f` / `right` | Move the query cursor |
| `ctrl-h` / `backspace`, `ctrl-d` / `delete` | Delete the character before or under the cursor |
| `ctrl-w`, `alt-backspace` | Delete the word before the cursor |
| `ctrl-u`, `ctrl-k` | Delete to the start or end of the query |

Selections are kept while you change the query, so you can select reminders from several searches before pressing Enter. In a menu, `esc` and `ctrl-g` go back to the prompt.

## Adding a reminder

When the query is plain text, the last row is `Add reminder: <query>`. Highlight it and press Enter to create the reminder with the [quick-add](quick-add.md) parser, then print its ID. A query with a `list:`, `due:`, `pri:`, `is:`, or `cal:` field or a `!` negation has no Add row, so Enter does nothing when it matches no reminders. The quick-add priorities `!low`, `!medium`, and `!high` don't count as negations among the trailing tokens quick-add reads, so `Pay rent !high` has an Add row and `!high Pay rent` doesn't. A query with no title before those tokens, such as `!high` alone, has no Add row. Because the text isn't parsed by a shell, `!high` and `#list` need no quotes:

```text
> Pay rent fri !high #bills
```

## Acting on reminders

`ctrl-o` opens a menu for the selected reminders, or for the highlighted one when nothing is selected:

| Key | Action |
| --- | --- |
| `x` | Mark completed, like `nib task done` |
| `p` | Set priority: `h` high, `m` medium, `l` low, or `n` none, like `nib task priority` |
| `m` | Move to a writable list, chosen by number or with the arrow keys and Enter, like `nib task move` |
| `d` | Delete, after confirming with `y`, or by typing the count and pressing Enter when more than three reminders are selected or any of them can't be fully backed up |

Each action backs up the reminders and writes a journal entry, as the [one-step commands](task-commands.md) do, so `nib log` lists it and `nib undo` reverses it. Deletes appear in the journal as `task_delete`. Undo can't bring back a deleted reminder itself: it re-creates the reminder with a new ID, so IDs printed earlier no longer work. nib can't back up alarms, repeats, or links it can't read, so undo re-creates those reminders without them. After an action, the prompt stays open with the updated reminders and a one-line result, and selections are cleared. If the change worked but nib couldn't reload the reminders or record the journal entry, the result line says so.

The action menus and the delete confirmation count selected reminders that the current query hides. When any selected reminder can't be fully backed up, the delete confirmation says how many and asks for the count. While typing the count, `backspace` corrects it, and any other typed character or `esc` cancels. Delete refuses reminders in read-only lists.

Editing reminders in `$EDITOR` from the prompt isn't available yet; use `nib task edit`.

## JSON

With `--json`, Enter prints an array of the chosen reminders instead of bare IDs. See [JSON output](json.md#picked-reminders).

## Building without the prompt

The prompt is part of the default `tui` Cargo feature. A build made with `--no-default-features` still accepts `nib pick` but reports that the terminal interface isn't available. Other commands don't load the terminal code, so the feature doesn't change their startup time.

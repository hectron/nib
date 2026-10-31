# Browse reminders on the board

`nib board`, or `nib b`, opens a full-screen board with three panes: lists on the left, items in the middle, and details on the right. Reminders can be filtered, added, edited, completed, reprioritized, moved, and deleted. Calendar events for today and the next six days are shown and filterable, but they are read-only.

```bash
nib board
NIB_DEMO=1 nib b
```

`NIB_DEMO=1 nib board` is read-only: the demo backend doesn't save changes, so actions report success but the reminders reload unchanged, and an added reminder disappears at the next reload.

The board draws on the terminal in the alternate screen and prints nothing when it closes, so `--json` has no effect. Standard input must be a terminal; otherwise `nib board` exits with `nib::board::not_a_terminal`. Scripts can use `nib task list --filter EXPR --json` and `nib cal --week --json` instead. Bare `nib` still prints help.

## Layout

- The top line shows the input state: `BROWSE`, `FILTER`, or `CAPTURE`, and today's date.
- The second line shows the filter, or the text being typed, with the result and selection counts.
- The third line shows the capture preview, a filter error, or the result of the last action.
- The lists pane has `All`, `Today`, each reminder list, `Events` when any events loaded, and then your saved views. Each row shows how many items it matches. Moving through the lists pane changes the items pane right away.
- The details pane shows a reminder's list, due date, priority, alarms, repeats, link, and notes, or an event's time, calendar, location, meeting link, and notes.
- The footer shows hints for the active state, built from your keymap.

Terminals narrower than 100 columns hide the details pane. Below 64 columns, only the focused pane is shown. `p` hides or shows the details pane.

## Input states

The board starts in BROWSE, where letters are actions. The main default keys are listed below; `?` or `f1` shows every key bound in the current state. See [Keymaps](keymap.md) to change them.

`alt-` keys need the Option key to send Meta. In Terminal, turn on Settings > Profiles > Keyboard > Use Option as Meta key. In iTerm2, set Settings > Profiles > Keys > General > Left Option key to Esc+. Otherwise Option types a symbol, such as `å` for `alt-a`, into FILTER or CAPTURE.

Pasted text is typed into the filter or capture line, and ignored in BROWSE.

### BROWSE

| Key | Action |
| --- | --- |
| `j` / `k`, `down` / `up` | Move in the focused pane |
| `g` / `G`, `page-up` / `page-down` | Move to the top or bottom, or by a page |
| `[` / `]`, `left` / `right` | Switch between the lists and items panes |
| `enter` | Focus the items pane from the lists pane |
| `/` | Enter FILTER |
| `a` or `n` | Enter CAPTURE |
| `space` or `tab` | Select or unselect the highlighted reminder |
| `alt-a` | Select every visible reminder, or unselect them when all are selected |
| `x`, `r` | Complete or reopen |
| `1`, `2`, `3`, `0` | Set priority high, medium, low, or none |
| `m` | Move to a writable list |
| `e` | Edit in `$VISUAL` or `$EDITOR` |
| `delete` or `backspace` | Delete, after a confirmation. The key labeled delete on a Mac keyboard sends `backspace` |
| `ctrl-o` | Open the action menu |
| `ctrl-l` | Clear the filter |
| `q`, `esc`, `ctrl-c` | Quit |

Actions apply to the selected reminders, or to the highlighted one when nothing is selected. Selections are kept while you change lists or the filter. After each reload, reminders that are no longer loaded drop out of the selection. If an action fails, the board reloads the reminders. If a reload fails, the selection is kept and the third line says it may be out of date.

### FILTER

`/` starts FILTER with the current filter. Typing filters with the same syntax as `nib task list --filter`: fuzzy title text plus `list:`, `due:`, `pri:`, `is:`, and `cal:` fields and `!` negation. See [Filtering reminders](filter.md). `is:event` shows only events, and completed reminders load the first time the filter contains `is:done`. An invalid filter is shown on the third line, and the results stay as they were until it is valid again.

| Key | Action |
| --- | --- |
| `enter` | Keep the filter and return to BROWSE |
| `esc` | Clear the filter and return to BROWSE |
| `ctrl-u` | Clear the typed filter and stay in FILTER |
| `alt-enter` | Keep the filter and return to BROWSE, even when the Add row is highlighted |
| `insert` | Add the typed text as a reminder |
| `up` / `down`, `ctrl-p` / `ctrl-n` | Move the highlight |
| `left` / `right` | Switch between the lists and items panes |
| `tab`, `shift-tab` | Select the highlighted reminder, and with `shift-tab` move up |
| `alt-a` | Select every visible reminder, or unselect them when all are selected |
| `ctrl-o` | Open the action menu |
| `ctrl-space` | Hide or show the details pane |
| `ctrl-q` | Quit |
| `backspace`, `ctrl-w`, `delete` | Delete a character, the word before the cursor, or the character under it |
| `ctrl-b` / `ctrl-f` | Move the cursor |

Because `left` and `right` switch panes in FILTER, use `ctrl-b` and `ctrl-f` to move the cursor.

Plain text adds an `Add reminder: <text>` row, as in [`nib pick`](pick.md#adding-a-reminder). Highlight it and press `enter`, or press `insert`, to add the text as a reminder. The filter is then cleared.

The filter combines with the highlighted list or view: a filter of `pri:high` under `Work` shows the high-priority reminders in Work.

### CAPTURE

`a` starts CAPTURE. Type a reminder in [quick-add](quick-add.md) form: a title, then a due date or time, `!low`, `!medium`, or `!high`, and `#list`. The third line previews how the text parses, including the list the reminder goes to, and shows the quick-add error when the text can't be added. No shell quoting is needed.

| Key | Action |
| --- | --- |
| `enter` | Add the reminder and return to BROWSE |
| `esc`, `ctrl-c` | Discard the text and return to BROWSE |
| `alt-h`, `alt-m`, `alt-l`, `alt-n` | Set the new reminder's priority to high, medium, low, or none |
| `up` / `down`, `ctrl-p` / `ctrl-n` | Move the item highlight |
| `ctrl-q` | Quit |

The priority keys don't change the text. They replace an inline `!priority` and `task.priority`, and they also work when `task.parse_inline` is `false`. The preview shows the result.

Without `#list`, the reminder goes to `task.list` or the Reminders default list, even when another list is highlighted.

## Menus and confirmations

`ctrl-o` opens a menu with `x` done, `r` reopen, `p` priority, `m` move, `e` edit, and `d` delete. Menus use fixed keys: a letter or digit chooses, `up` and `down` with `enter` also choose, and `esc`, `ctrl-g`, or `ctrl-c` go back. These keys don't come from the keymap.

Delete asks for `y`, or for the count typed and `enter` when more than three reminders are selected, when any of them can't be fully backed up, or when one is no longer loaded. The confirmation counts selected reminders hidden by the current list or filter. If a reminder gained an alarm, repeat, or link after the board loaded it, nib deletes nothing, reloads, and asks for the count. These are the same safeguards as `nib pick`.

Each change backs up the reminders and writes a journal entry, as the [one-step commands](task-commands.md) do, so `nib log` lists it and `nib undo` reverses it.

## Editing in your editor

`e` opens the selected reminders, or the highlighted one, in `$VISUAL` or `$EDITOR` through the [`task edit`](task-edit.md) flow. The board leaves the alternate screen while the editor runs. The file holds only those reminders under every list heading, so you can also move them by moving lines. The change plan and the confirmation prompt appear on the normal terminal, followed by the usual backup and journal entry. The board then returns with the result on its third line and reloads the reminders.

`task edit` refuses a file with no reminders left, so to delete the only reminder in the file, use `delete` on the board instead. Completed reminders are left out of the file.

## Saved views

Saved views are named filter expressions in any configuration file:

```toml
[tui.views]
"Work, high" = "list:work pri:high"
Overdue = "is:overdue"
"This week" = "due:week !list:someday"
```

They appear after the lists, in name order. A nearer configuration file replaces a view with the same name. Each expression is checked with the filter parser when the configuration loads, so a typo is reported with the file and key, such as `tui.views."Overdue"`, and `nib doctor` reports it as a configuration error.

## What is read-only

Events are never changed. Pressing an action key on an event says `Events are read-only.` Selecting all skips events. Apple Notes aren't shown.

## Building without the board

The board is part of the default `tui` Cargo feature. A build made with `--no-default-features` still accepts `nib board` but reports `nib::board::unavailable`. Other commands don't load the terminal code, and the board loads reminders and events only when it starts.

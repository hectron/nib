# Keymaps

[`nib board`](board.md) reads these settings: every key in BROWSE, FILTER, and CAPTURE runs the bound actions, and the board's footer and `?` help are generated from the active keymap. `nib pick` keeps its own fixed keys, listed in [Picking reminders](pick.md#keys). `nib doctor` validates the keymap.

Bindings are grouped by input state:

- `browse` is for navigation and reminder actions.
- `filter` is for typing filter text while navigation and selection keys stay available.
- `capture` is for typing a new reminder while submission and priority keys stay available.

The `default` preset is the base keymap. Override individual chords in any nib configuration file:

```toml
[tui]
preset = "default"

[tui.keys.browse]
"x" = "done"

[tui.keys.filter]
"ctrl-o" = "act-menu"
"alt-x" = ["done", "down"]
```

A string runs one action. An array runs actions from left to right. Set a chord to `"none"` to remove its preset or lower-precedence binding:

```toml
[tui.keys.browse]
"q" = "none"
```

Key tables merge chord by chord through the normal configuration precedence. A nearer file replaces only the chords it defines. Keymap tables are edited by hand; `nib config set` and `nib config unset` handle scalar settings such as `tui.preset`.

`nib doctor` validates the active keymap. It reports invalid presets, states, chords, actions, canonical duplicate chords, and states without a `quit` or `cancel` binding.

## How the board uses keys

In FILTER and CAPTURE, a key that has no binding types or edits the text: printable characters insert, `backspace` and `ctrl-h` delete the previous character, `delete` deletes the next one, `ctrl-w` and `alt-backspace` delete the previous word, and `left`, `right`, `ctrl-b`, and `ctrl-f` move the cursor when they aren't bound. In BROWSE, an unbound key does nothing.

On the board, the actions do this:

- `up`, `down`, `page-up`, `page-down`, `top`, and `bottom` move in the focused pane, which is the lists or the items pane. In CAPTURE, `up`, `down`, `page-up`, and `page-down` move the item highlight.
- `filter` enters FILTER with the current filter; `capture` and `add` enter CAPTURE from BROWSE. In FILTER, `add` adds the typed text as a reminder when it is plain text.
- In FILTER, `accept` keeps the filter, or adds the reminder when the Add row is highlighted, and `keep-filter` always keeps it; `cancel` clears the filter; `clear-filter` empties the typed text. In BROWSE, `clear-filter` clears the kept filter.
- In CAPTURE, `accept` adds the reminder, `cancel` discards the text, and the `priority-*` actions set the new reminder's priority, replacing any inline `!priority` and `task.priority`. Elsewhere they change the priority of the targets. Other actions, such as `next-pane`, `act-menu`, and `done`, do nothing in CAPTURE.
- `done`, `reopen`, `move`, `edit`, and `delete` act on the selected reminders, or on the highlighted one.
- `accept` in BROWSE focuses the items pane from the lists pane; `cancel` in BROWSE clears the selection.
- `next-pane` and `prev-pane` switch between the lists and items panes, and `preview-toggle` hides or shows the details pane.
- `help` shows the generated key help for the current state, and `quit` closes the board.

Action menus and the delete confirmation use fixed keys; see [Menus and confirmations](board.md#menus-and-confirmations).

## Key chords

Printable characters use their literal form, such as `x`, `/`, or `?`. Named keys include `enter`, `esc`, `tab`, `up`, `down`, `left`, `right`, `home`, `end`, `page-up`, `page-down`, `backspace`, `delete`, `insert`, `f1` through `f24`, and `space`. Write function keys without leading zeros, such as `f1`, not `f01`.

Prefix a key with `ctrl-`, `alt-`, or `shift-`. Modifiers and named keys are case-insensitive and are shown canonically in `ctrl-alt-shift` order. A bare uppercase ASCII letter is its shifted form, so `G` and `shift-g` are the same chord. Letter case after `ctrl-` or `alt-` is ignored, so `Ctrl-O` and `ctrl-o` are also the same chord and cannot both occur in one state table.

## Actions

| Action | Effect |
| --- | --- |
| `up`, `down` | Move by one item. |
| `page-up`, `page-down` | Move by one page. |
| `top`, `bottom` | Move to the first or last item. |
| `filter`, `capture` | Enter the named input state. |
| `clear-filter`, `keep-filter` | Clear or preserve the current filter. |
| `select-toggle`, `select-all` | Change the visible selection. |
| `act-menu` | Open the action menu. |
| `done`, `reopen` | Complete or reopen selected reminders. |
| `priority-high`, `priority-medium`, `priority-low`, `priority-none` | Change reminder priority. |
| `move`, `edit`, `delete`, `add` | Run the named reminder action. |
| `quit`, `help`, `accept`, `cancel` | Control the current interface or input state. |
| `next-pane`, `prev-pane` | Move focus between panes. |
| `preview-toggle` | Show or hide the preview. |
| `none` | Remove a binding. It must be the only action for that chord. |

## Default preset

The defaults combine fzf and Telescope selection conventions, Emacs/readline control keys, and Vim-style browse keys.

### Browse

| Chords | Actions |
| --- | --- |
| `up`, `k`, `ctrl-p` | `up` |
| `down`, `j`, `ctrl-n` | `down` |
| `page-up`, `ctrl-u` | `page-up` |
| `page-down`, `ctrl-d` | `page-down` |
| `home`, `g`, `ctrl-a` | `top` |
| `end`, `G`, `ctrl-e` | `bottom` |
| `/` | `filter` |
| `a` | `capture` |
| `ctrl-l` | `clear-filter` |
| `alt-k` | `keep-filter` |
| `space`, `tab` | `select-toggle` |
| `alt-a` | `select-all` |
| `ctrl-o` | `act-menu` |
| `x` | `done` |
| `r` | `reopen` |
| `1`, `2`, `3`, `0` | `priority-high`, `priority-medium`, `priority-low`, `priority-none` |
| `m`, `e`, `n` | `move`, `edit`, `add` |
| `delete`, `backspace` | `delete` |
| `q`, `esc`, `ctrl-c` | `quit` |
| `?`, `f1` | `help` |
| `enter` | `accept` |
| `right`, `]` | `next-pane` |
| `shift-tab` | `select-toggle`, then `up` |
| `left`, `[` | `prev-pane` |
| `p` | `preview-toggle` |

### Filter

| Chords | Actions |
| --- | --- |
| `up`, `ctrl-p` | `up` |
| `down`, `ctrl-n` | `down` |
| `page-up`, `page-down` | `page-up`, `page-down` |
| `home`, `ctrl-a` | `top` |
| `end`, `ctrl-e` | `bottom` |
| `ctrl-u` | `clear-filter` |
| `alt-enter` | `keep-filter` |
| `tab` | `select-toggle` |
| `shift-tab` | `select-toggle`, then `up` |
| `alt-a` | `select-all` |
| `ctrl-o` | `act-menu` |
| `insert` | `add` |
| `enter` | `accept` |
| `esc`, `ctrl-c` | `cancel` |
| `ctrl-q` | `quit` |
| `f1` | `help` |
| `right`, `left` | `next-pane`, `prev-pane` |
| `ctrl-space` | `preview-toggle` |

### Capture

| Chords | Actions |
| --- | --- |
| `up`, `ctrl-p` | `up` |
| `down`, `ctrl-n` | `down` |
| `enter` | `accept` |
| `esc`, `ctrl-c` | `cancel` |
| `ctrl-q` | `quit` |
| `f1` | `help` |
| `alt-h`, `alt-m`, `alt-l`, `alt-n` | `priority-high`, `priority-medium`, `priority-low`, `priority-none` |

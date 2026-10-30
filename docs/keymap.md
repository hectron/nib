# Keymaps

nib has three input states:

- `browse` interprets keys as navigation and reminder actions.
- `filter` accepts filter text while keeping navigation and selection keys available.
- `capture` accepts a new reminder while keeping submission and priority keys available.

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

`nib doctor` validates the active keymap. It reports invalid presets, states, chords, actions, canonical duplicate chords, and states without a `quit` or `cancel` binding. The interactive help is generated from the active keymap, so overrides appear there automatically.

## Key chords

Printable characters use their literal form, such as `x`, `/`, or `?`. Named keys include `enter`, `esc`, `tab`, `up`, `down`, `left`, `right`, `home`, `end`, `page-up`, `page-down`, `backspace`, `delete`, `insert`, `f1` through `f24`, and `space`.

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
| `m`, `e`, `delete`, `n` | `move`, `edit`, `delete`, `add` |
| `q`, `esc`, `ctrl-c` | `quit` |
| `?`, `f1` | `help` |
| `enter` | `accept` |
| `right`, `]` | `next-pane` |
| `left`, `[`, `shift-tab` | `prev-pane` |
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
| `tab`, `shift-tab` | `next-pane`, `prev-pane` |
| `ctrl-o` | `act-menu` |
| `alt-h`, `alt-m`, `alt-l`, `alt-n` | `priority-high`, `priority-medium`, `priority-low`, `priority-none` |

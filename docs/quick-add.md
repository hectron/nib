# Quick-add reminders

Add a reminder with a title followed by a due date, priority, or list:

```bash
nib t Pay rent fri !high '#bills'
```

This creates “Pay rent,” due Friday, with high priority in the existing `bills` list. The same trailing tokens work with `nib task add`.

## Due dates

Use natural dates inline or with `--due`:

| Form | Meaning |
| --- | --- |
| `today`, `tomorrow` | Today or tomorrow |
| `fri`, `friday` | The nearest matching weekday, including today |
| `next fri` | Seven days after the bare weekday result |
| `in 3d`, `+3d` | A number of days from today |
| `in 2w`, `+2w` | A number of weeks from today |
| `eom` | The last day of this month |
| `2027-01-05` | An exact date in `YYYY-MM-DD` form |

All weekdays accept their three-letter or full English names. For example, on Friday, `fri` means today and `next fri` means the following Friday.

```bash
nib t Submit expenses --due "next fri"
nib task add Renew passport --due +2w
```

## Trailing tokens

Quick-add reads these fields only from the end of the title:

- A due date in any form above.
- `!low`, `!medium`, or `!high` for priority.
- `#NAME` for a reminder list.

The tokens can appear in any order. Quote a `#NAME` token so your shell does not treat it as a comment. Explicit flags take precedence over inline fields:

```bash
nib t Pay rent fri !low '#bills' --due tomorrow --priority high --list home
```

That reminder is due tomorrow, with high priority in the `home` list.

To keep a recognized trailing token in the title, prefix it with a backslash. Quote the token so the shell passes that backslash to nib:

```bash
nib t Discuss '\fri'
nib t Choose '\!high'
nib t Label '\#bills'
```

These titles end in `fri`, `!high`, and `#bills`; none of those tokens sets a field. Inline parsing is enabled by default and can be disabled in [configuration](configuration.md).

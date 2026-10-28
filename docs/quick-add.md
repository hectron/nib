# Quick-add reminders

Add a reminder with a title followed by a due date, priority, or list:

```bash
nib t Pay rent fri '!high' '#bills'
```

This creates “Pay rent,” due Friday, with high priority in the existing `bills` list. The same trailing tokens work with `nib task add`.

## Due dates and times

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
| `tomorrow 09:00` | Tomorrow at 09:00 |
| `fri 9am` | Friday at 09:00 |
| `9:30pm` | Today at 21:30 |
| `2026-11-03T09:00` | An exact date and time |
| `2026-11-03 09:00` | An exact date followed by a time |

All weekdays accept their three-letter or full English names. For example, on Friday, `fri` means today and `next fri` means the following Friday.

```bash
nib t Submit expenses --due "next fri"
nib task add Renew passport --due +2w
nib t Call London --due "tomorrow 09:00"
```

Times accept `HH:MM`, `Ham`, or `H:MMam` forms, case-insensitively. A time token is recognized only immediately after a date token or by itself at the end, where it means today. Times supplied by quick-add or `--due` use the local time zone.

## Trailing tokens

Quick-add reads these fields only from the end of the title:

- A due date or time in any form above.
- `!low`, `!medium`, or `!high` for priority.
- `#NAME` for a reminder list.

The tokens can appear in any order. Quote `!PRIORITY` and `#NAME` tokens so your shell does not expand `!` as history or treat `#` as a comment. Explicit flags take precedence over inline fields:

```bash
nib t Pay rent fri '!low' '#bills' --due tomorrow --priority high --list home
```

That reminder is due tomorrow, with high priority in the `home` list.

The shell also parses quotes and other special characters in titles before nib starts. Double-quote a title containing an apostrophe, then leave due-date tokens outside the title so quick-add can recognize them:

```bash
nib t "Get Apple Cider from the Farmer's Market" tomorrow '!high'
```

An unmatched apostrophe leaves an interactive shell waiting for the rest of a quote; nib cannot report that error because it has not been started yet.

To keep a recognized trailing token in the title, prefix it with a backslash. Quote the token so the shell passes that backslash to nib:

```bash
nib t Discuss '\fri'
nib t Choose '\!high'
nib t Label '\#bills'
nib t Pay '\next' fri 9am
nib t Keep '\tomorrow' 09:00
nib t Keep '\9:30pm'
nib t Keep '\2026-11-03T09:00'
```

These titles end in `fri`, `!high`, `#bills`, `next fri 9am`, `tomorrow 09:00`, `9:30pm`, and `2026-11-03T09:00`; none of those tokens sets a field. Escape the first word of a multi-token date or date-time, and escape a standalone time or `T` form directly. A backslash before any other word, such as `'\server'`, stays in the title. Inline parsing is enabled by default and can be disabled in [configuration](configuration.md).

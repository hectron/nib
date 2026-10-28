## [0.4.1] - 2026-10-28

### 🐛 Bug Fixes

- Keep config comments on unset, make cleanup best effort, report stale locks
- Keep comments across end-of-file and kept-section config unsets
- Release the journal lock during prompts and surface real re-check errors
- Compare full entry contents when resolving a pending undo
- Pin the earlier offset for due times in a daylight saving gap

### 🧪 Testing

- Snapshot mixed doctor locks and pin set-after-unset comment order
- Tighten the pending undo resolve tests
- Cover fall-back times and gap display for unnamed zones
## [0.4.0] - 2026-10-27

### 🚀 Features

- Show and edit reminder due times
- Add shell completions with live list, calendar, and folder names
- Add `nib config` to list, get, set, and unset settings

### 🐛 Bug Fixes

- Journal edits under the lock and refuse unrecorded undos clearly

### ⚙️ Miscellaneous Tasks

- *(release)* V0.4.0
## [0.3.0] - 2026-10-27

### 🚀 Features

- Add `nib log` and `nib undo` for task edits
- Add `nib doctor` to check access and configuration
- Color tables and JSON in the terminal

### 🐛 Bug Fixes

- Make `nib undo` safe to retry and keep demo mode off disk
- Recover safely from interrupted or partly recorded undos
- Require an explicit answer to resolve a pending undo
- Keep unrelated backslashes and `2026-plan` in quick-add titles
- Reject ambiguous Notes folders and batch note listing

### 📚 Documentation

- Render the demo GIF in Rosé Pine Dawn with TX-02
- Suggest a starter config in the README
- Show colored output in the demo GIF

### ⚙️ Miscellaneous Tasks

- Report the required check from the release workflow
- *(release)* V0.3.0

### 💼 Other

- Upgrade Rust to 1.99.1
## [0.2.0] - 2026-10-27

### 🚀 Features

- Add `nib today` agenda view
- Parse quick-add suffixes and natural-language dates
- Capture and list Apple Notes through AppleScript

### 🐛 Bug Fixes

- Restore the Date import that `nib today` needs

### 📚 Documentation

- Restructure the docs around a short README
- Cover Notes, quick-add, and `nib today`, and add the demo GIF

### 🚜 Refactor

- Adopt STYLE.md, strict lints, and a layered library structure

### ⚙️ Miscellaneous Tasks

- Cut releases with one workflow_dispatch command
- Release through a pull request with two commands
- *(release)* V0.2.0

### 💼 Other

- Pin the Rust toolchain to 1.98.1
## [0.1.0] - 2026-10-27

### 🚀 Features

- Add command-line scaffold for notes, tasks, and calendar
- Return typed backend errors with diagnostic codes
- Build each backend on first use
- Add stable identifiers to notes, reminders, and events
- Parse --due into a date resolved from the current time
- Give event starts a time zone
- Pass the calendar a concrete time range instead of Span
- Add a demo backend behind NIB_DEMO
- Read reminders from EventKit
- Add reminders through EventKit
- List calendar events from EventKit
- Show all-day events as all day
- Convert reminders to and from a Markdown task list
- Edit reminders as Markdown with `nib task edit`
- Layer configuration from flags, env, project files, and user config

### 🐛 Bug Fixes

- Make n and t capture commands so subcommands never become titles
- Render tables the same in and out of a terminal
- Strip control characters from text output

### 📚 Documentation

- Add README and agent guidelines
- Document capture commands and output rules
- Note that backends are built on first use
- Add a rule for keeping the JSON shape stable
- Describe date parsing and the clock rule
- Document Reminders, macOS 26, and terminal permissions
- Document Calendar, event ids, and CLI test limits
- Document `nib task edit`
- Add a git-cliff changelog and release steps

### ⚡ Performance

- Read the local time zone without scanning zoneinfo

### 🚜 Refactor

- Pass note, reminder, and event requests as structs
- Build backends from factory closures
- Let the command choose the summary verb
- Serialize JSON through dedicated output types
- Rename the project from jot to nib

### 🧪 Testing

- Cover options before subcommands and multi-word titles
- Cover control characters in text output
- Expect a diagnostic code from stub backend errors
- Check that each command builds only its backend
- Show that the caller can choose the summary verb
- Snapshot the note list, added task, and event list JSON
- Expect an id first in every JSON object
- Expect --due to reject anything that isn't a date
- Expect event starts in JSON as ISO 8601 with an offset
- Expect NIB_DEMO to show sample data
- Round-trip a real reminder through the Markdown file

### ⚙️ Miscellaneous Tasks

- License nib under MIT
- Check fmt, clippy, and tests on macOS 26
- Release with cargo-dist to GitHub and Homebrew
- Publish GitHub releases only for now
- *(release)* V0.1.0

### 💼 Other

- Target macOS 26 and add the EventKit bindings

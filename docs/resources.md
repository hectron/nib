# Folders, lists, and calendars

Discover the resource names that `nib` accepts before creating or filtering items:

```bash
nib note folders
nib task lists
nib cal calendars
```

The text tables include the Notes account or EventKit source so resources with the same name can be distinguished. Reminder lists and calendars also show whether they are writable and which one is the default. Use `--json` to include each resource's identifier:

```bash
nib --json note folders
nib --json task lists
nib --json cal calendars
```

The `type` field for reminder lists and calendars is EventKit's calendar type: `local`, `caldav`, `exchange`, `subscription`, `birthday`, or `unknown`. It is not the Reminders app's view layout. Apple's public EventKit API does not expose whether a list uses the standard list, sections, or columns view.

## Create and rename resources

Create a resource in a named account or source:

```bash
nib note folders create "Project Notes" --account iCloud
nib task lists create Errands --source iCloud
nib cal calendars create Releases --source iCloud
```

The owner selector accepts either the account or source name shown by the discovery command, or its stable identifier from `--json`. If you omit it, Notes uses its default account. Reminders and Calendar prefer the source of the current default list or calendar, then use the only eligible source when there is exactly one. To fail safely, nib considers an EventKit source eligible only after EventKit reports an existing mutable resource for the same entity type; use the Apple app to create the first list or calendar in a new source. `nib` reports an ambiguity instead of guessing when it cannot choose safely.

Rename a resource by its exact name, with an owner selector when duplicate names exist:

```bash
nib note folders rename "Project Notes" Projects --account iCloud
nib task lists rename Errands Shopping --source iCloud
nib cal calendars rename Releases Deployments --source iCloud
```

Use `--id` when the first argument is the resource's stable identifier instead of its name:

```bash
nib task lists rename 4C92A1 Shopping --id
```

`--id` cannot be combined with `--account` or `--source`, because the resource identifier is already unambiguous. Add the global `--json` option to a create or rename command to receive the changed resource object instead of a summary. In demo mode, these commands return representative output but do not persist changes.

Notes folder names managed by nib must be unique across the whole account, including nested folders. The CLI does not expose a parent-folder selector, so allowing a duplicate at a different depth would make later name-based commands ambiguous. Use `--id` to rename an existing folder whose current name has surrounding whitespace or another value that cannot be typed safely as a selector.

Deleting folders, lists, and calendars is intentionally outside this first management release. These resources can contain user data, so `nib` will not add destructive commands until it can provide consistent previews, confirmation, and recovery guidance across all three Apple apps. Use the Apple app when you need to delete one.

Apple Notes does not publish folder mutability metadata through its scripting dictionary. `nib` therefore cannot reliably pre-classify every special or system folder: Notes remains the authority, and any rejected rename is reported with the scripting error. Live mutation tests are opt-in and require `NIB_LIVE_WRITE=1`; the default test suite never writes to Notes.

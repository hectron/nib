# Contributing

nib is a Rust CLI for macOS 26 and later. Install the stable Rust toolchain with `rustfmt` and Clippy, then build the checkout:

```bash
git clone https://github.com/hectron/nib.git
cd nib
cargo build
cargo run -- --help
```

Use the demo backend while developing commands. It never reads or writes Apple data:

```bash
NIB_DEMO=1 cargo run -- t
NIB_DEMO=1 cargo run -- cal --week
NIB_DEMO=1 cargo run -- note list
```

## Checks

Run the same checks expected before a change is handed off:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

The default suite contains unit, snapshot, and CLI tests. It must not touch real Apple data. CLI tests use the demo backend or a stub that exits before requesting access.

Command output is covered by [insta](https://insta.rs/) snapshots. Review every intended snapshot change before accepting it:

```bash
cargo insta review
```

## Live Apple tests

Live tests are ignored by default. Run them only on macOS from a terminal that already has the required Reminders, Calendar, or Apple Notes permission. Notes access requires allowing the terminal to control Notes under **System Settings > Privacy & Security > Automation**.

Read-only tests:

```bash
cargo test lists_real_events -- --ignored
cargo test lists_real_reminders -- --ignored
```

Write tests also require `NIB_LIVE_WRITE=1`. They create a reminder or note and remove it again:

```bash
NIB_LIVE_WRITE=1 cargo test adds_and_removes_a_real_reminder -- --ignored
NIB_LIVE_WRITE=1 cargo test edits_a_real_reminder_through_markdown -- --ignored
NIB_LIVE_WRITE=1 cargo test live_create_list_and_delete_tricky_title -- --ignored
```

Do not run ignored tests as part of the normal suite.

## Project rules

Read [STYLE.md](STYLE.md) before changing code. It defines module boundaries, naming, diagnostics, unsafe-code rules, formatting, and lint policy.

Release automation and maintainer commands are documented in [RELEASING.md](RELEASING.md). Pull requests should not edit release history or create tags.

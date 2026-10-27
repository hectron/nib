# Style guide

`nib` follows the Rust Style Guide, the Rust API Guidelines, Comprehensive
Rust's unsafe discipline, and Reintech's guidance for structuring large Rust
projects. The rules below turn those sources into checks that apply to this
repository.

## Formatting and source layout

- Run `cargo fmt`; checked-in Rust must match stable `rustfmt` output.
- Keep imports, expressions, and declarations in the form chosen by `rustfmt`.
- Do not align code manually or use formatting that fights `rustfmt`.
- Keep tests beside the code they exercise. Put only cross-crate behavior in
  `tests/`.
- Do not add inline comments. The only inline comments permitted are required
  `// SAFETY:` comments on unsafe blocks.

## Naming and public API

- Follow the Rust API Guidelines: types and traits use `UpperCamelCase`,
  functions, methods, modules, and variables use `snake_case`, and constants
  use `SCREAMING_SNAKE_CASE`.
- Names communicate role rather than implementation detail. Conversions follow
  the conventional `as_`, `to_`, and `into_` meanings.
- `src/lib.rs` is a curated facade. New modules are private by default; expose
  only entry points and types that an external caller needs.
- Public behavior includes JSON field names, command output, exit behavior, and
  diagnostic codes. Change it deliberately and cover it with snapshots.

## Architecture and errors

- Preserve the layer direction: `domain` is pure types, traits, and errors;
  `app` contains use cases and pure editing logic; `infra` owns filesystem,
  process, environment, Apple-framework, and other I/O; `cli` parses arguments
  and renders output.
- Domain code does not import `app`, `infra`, or `cli`; infrastructure
  implements domain traits; cross-layer calls use narrow, explicit APIs or
  traits rather than shared mutable state.
- Keep related code in focused modules rather than growing a catch-all module.
- Define recoverable, structured errors with `thiserror`; add `miette`
  diagnostics where users need codes or remediation.
- Do not use `unwrap` outside tests. Outside tests, `expect` is reserved for a
  proved invariant and its message must state why failure is impossible.

## Unsafe code

- Unsafe code is denied crate-wide and allowed only in `infra::eventkit`, the
  Objective-C boundary.
- Wrap Objective-C operations in small safe helpers so callers cannot violate
  their preconditions.
- Every unsafe block has an immediately preceding `// SAFETY:` comment that
  states the invariant making the operation sound.
- Never send an Objective-C object across threads. Convert it to owned Rust
  values inside the EventKit callback.
- `unsafe_op_in_unsafe_fn` is denied, so an unsafe function does not implicitly
  permit unsafe operations in its body.

## Lints and verification

- Crate lint policy denies unsafe code, undocumented unsafe blocks, and
  `unwrap`; it enables Clippy's `pedantic` group as warnings.
- Tests may use `unwrap` and `expect` because a panic is the intended assertion
  mechanism and the failing expression is reported by the test harness.
- `module_name_repetitions` is allowed because explicit names such as
  `ConfigError` and `ReminderSnapshot` remain clear at call sites.
- `must_use_candidate` is allowed because `#[must_use]` is an API-contract
  decision, not a mechanical requirement for every pure helper.
- `missing_errors_doc` and `missing_panics_doc` are allowed while the crate's
  narrow public facade is documented by behavior and diagnostics rather than
  repetitive boilerplate sections.
- Before handoff, run `cargo fmt`,
  `cargo clippy --all-targets -- -D warnings`, and `cargo test`. Do not run
  ignored tests unless the task explicitly calls for live Apple data.

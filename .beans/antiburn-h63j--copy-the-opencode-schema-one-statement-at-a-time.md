---
# antiburn-h63j
title: Copy the OpenCode schema one statement at a time
status: completed
type: task
priority: low
tags:
    - remote
    - security
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T01:37:57Z
parent: antiburn-gt1g
---

`opencode_extract` (`crates/antiburn-remote/src/database.rs:74,125`) runs the `sql` text from the source database's `sqlite_master` through `execute_batch`. That runs every statement in the text on a read-write `:memory:` connection, where `ATTACH` could create a file. Only someone who can write `opencode.db` can do this, so this is hardening.

- Accept only text that starts with `CREATE TABLE` or `CREATE INDEX`, and run it with `execute` (one statement).
- Test: schema text with a second statement is rejected.
- AC: a second statement in the source schema never runs.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Removed rusqlite extra_check and restored all six fixture files changed for that feature. copy_schema_statement now prepares locally, compares expanded_sql with the input while ignoring trailing whitespace and semicolons, rejects additional statement text with its own error, and executes the prepared statement. CREATE TABLE / CREATE INDEX validation remains.

Both schema tests pass, including injected ATTACH statements in table and index definitions with assertions that no marker file exists. Added acceptance cases for trailing semicolon/whitespace and semicolons inside quoted identifiers.

Remote nextest: 89 passed. Remote clippy --all-targets -- -D warnings and cargo fmt --all --check passed. Final bob check passed on signed commit 17fb1e72, including desktop src-tauri tests with restored fixtures. Bean acceptance items remain checked. Nothing pushed.

---
# antiburn-h63j
title: Copy the OpenCode schema one statement at a time
status: todo
type: task
priority: low
tags:
    - remote
    - security
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`opencode_extract` (`crates/antiburn-remote/src/database.rs:74,125`) runs the `sql` text from the source database's `sqlite_master` through `execute_batch`. That runs every statement in the text on a read-write `:memory:` connection, where `ATTACH` could create a file. Only someone who can write `opencode.db` can do this, so this is hardening.

- Accept only text that starts with `CREATE TABLE` or `CREATE INDEX`, and run it with `execute` (one statement).
- Test: schema text with a second statement is rejected.
- AC: a second statement in the source schema never runs.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

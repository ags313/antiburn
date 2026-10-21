---
# antiburn-s0p7
title: Read the store revision off the async runtime
status: todo
type: bug
priority: low
tags:
    - store
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`apps/desktop/src-tauri/src/commands/mod.rs:1453` calls `Store::revision()` in an async command. `revision()` takes the writer mutex, so a long write transaction blocks a tokio worker thread.

- Read the revision inside the existing `run_blocking` closure, or read it without the writer lock.
- AC: the async path takes no writer-mutex lock.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

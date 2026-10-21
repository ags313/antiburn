---
# antiburn-yanj
title: Open remote SQLite copies in defensive mode
status: todo
type: task
priority: low
tags:
    - remote
    - security
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

The desktop opens OpenCode extracts and Antigravity snapshots that a remote host supplied, with only `SQLITE_OPEN_READ_ONLY`. `PRAGMA table_info` accepts views, so a hostile copy can name a recursive view `message` and hang the analysis worker.

- Set `trusted_schema=OFF` and `SQLITE_DBCONFIG_DEFENSIVE`, require the needed names to be tables in `sqlite_master`, and add a progress handler or time limit.
- Apply this to remote cache copies; apply it to all provider database reads if that costs nothing extra.
- Test: a copy with a view named `message` is rejected quickly.
- AC: a hostile copy cannot hang the analysis worker.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

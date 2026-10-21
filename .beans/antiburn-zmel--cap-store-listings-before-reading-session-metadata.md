---
# antiburn-zmel
title: Cap store listings before reading session metadata
status: todo
type: bug
priority: normal
tags:
    - remote
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`stores.rs:95` calls `entry_for` (a `session_log_read`) for every log in the window before the 200/1,000 cap applies. Cursor discovery also reads every transcript. With All history, each sync reads every session on the host.

- Sort by modification time and apply the cap first; read metadata only for the kept logs.
- Test with a fake explorer that counts metadata reads.
- AC: a listing reads metadata for at most the cap's number of sessions.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

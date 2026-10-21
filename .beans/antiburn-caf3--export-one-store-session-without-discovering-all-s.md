---
# antiburn-caf3
title: Export one store session without discovering all sessions
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

`stores::prepare` (`crates/antiburn-remote/src/stores.rs:233-242`) runs full discovery and metadata reads for the agent, then filters by session id. Each export is O(n), so an All-history backfill is O(n²) and can hold several GB in memory on the remote host.

- Find the target from its id: OpenCode and Antigravity rows by id, Cursor and Junie paths from the session's listing entry. Admit the found files through the same path checks as today.
- Keep the manifest and path validation unchanged.
- Test with a fake explorer that counts reads: one export reads only that session's files.
- AC: exporting one session reads only that session's files.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

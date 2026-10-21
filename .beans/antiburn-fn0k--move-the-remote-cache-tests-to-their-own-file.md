---
# antiburn-fn0k
title: Move the remote_cache tests to their own file
status: todo
type: task
priority: low
tags:
    - desktop
    - refactor
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`apps/desktop/src-tauri/src/remote_cache.rs` grew from 1,013 to 1,411 lines; about 330 of them are inline tests, and the production code ends near line 696.

- Move the tests to `remote_cache/tests.rs`, as `scan` and `store` do.
- AC: `remote_cache.rs` is under 1,000 lines, and the tests pass.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

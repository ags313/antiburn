---
# antiburn-69qq
title: Store the remote cache generation directory explicitly
status: todo
type: task
priority: normal
tags:
    - remote
    - desktop
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
blocked_by:
    - antiburn-79ny
---

`generation_dir` (`apps/desktop/src-tauri/src/remote_cache.rs:45`) finds the cache directory by counting path components of `primary_file_name`, and `cached_source` (`:56`) matches agents again. If a layout changes, prune and delete act on the wrong directory.

- Record the generation directory in the cache record; let `cached_source` read the source kind from the shared table.
- Test prune and delete with a nested layout.
- AC: prune and delete use the recorded directory.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

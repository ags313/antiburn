---
# antiburn-69qq
title: Store the remote cache generation directory explicitly
status: completed
type: task
priority: normal
tags:
    - remote
    - desktop
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T00:43:38Z
parent: antiburn-gt1g
blocked_by:
    - antiburn-79ny
---

`generation_dir` (`apps/desktop/src-tauri/src/remote_cache.rs:45`) finds the cache directory by counting path components of `primary_file_name`, and `cached_source` (`:56`) matches agents again. If a layout changes, prune and delete act on the wrong directory.

- Record the generation directory in the cache record; let `cached_source` read the source kind from the shared table.
- Test prune and delete with a nested layout.
- AC: prune and delete use the recorded directory.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Fixed the P1: a stored row with no recorded directory (a copy from an earlier build) no longer has its cache cleared before the export, so a failed export leaves the old copy readable.

- A new `clear_before_export` skips the pre-export deletion when a row is stored without a recorded directory. A new session or a recorded directory still clears stray generations as before.
- After the replacement and its directory commit, `remove_generations_except(parent, target)` removes every other generation. That includes the legacy one.
- Regression test `a_failed_sync_keeps_the_cache_of_an_unrecorded_legacy_record`: an upgraded row with a NULL directory runs the pre-export step, and its file is still readable. The same test then checks that a committed replacement removes the legacy generation.
- The nested-layout prune/delete test is unchanged.
- Results: `cargo nextest run` gives 3264 passed and 5 failed, and the 5 are the known managed-settings tests. Clippy with `-D warnings` and `cargo fmt --check` are clean. `bob check` passes.

The test runs the pre-export step directly, not a full `sync_session` with a failing SSH export, because `sync_session` needs a Tauri `AppHandle` and SSH.

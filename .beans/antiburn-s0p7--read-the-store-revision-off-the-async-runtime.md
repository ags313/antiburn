---
# antiburn-s0p7
title: Read the store revision off the async runtime
status: completed
type: bug
priority: low
tags:
    - store
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T02:05:02Z
parent: antiburn-gt1g
---

`apps/desktop/src-tauri/src/commands/mod.rs:1453` calls `Store::revision()` in an async command. `revision()` takes the writer mutex, so a long write transaction blocks a tokio worker thread.

- Read the revision inside the existing `run_blocking` closure, or read it without the writer lock.
- AC: the async path takes no writer-mutex lock.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Validation: the revision-dispatch regression and existing runtime-responsiveness
tests pass. Desktop clippy with `-D warnings` and formatting pass. The direct
nextest command reaches a documented managed-settings baseline failure; an
isolated run excluding bob's known failures also encounters three sandbox
filesystem-watcher timeouts. The required `bob check` passes build and test
checks with its isolated HOME and documented baseline exclusions.

## Summary of Changes

Moved the Checks report Store::revision() read into the existing run_blocking snapshot closure. The async path no longer takes the writer mutex for this read. Added a regression test for the dispatch boundary; it and both existing runtime-responsiveness tests pass.

Rebased onto feat/all-usage-features first (already current). All acceptance items are checked in the bean and committed with DCO sign-offs. Final commit: 7e8064ed. No push performed.

Desktop clippy --all-targets -- -D warnings and cargo fmt --check pass. The exact requested nextest command encounters a documented baseline managed-settings failure; an isolated run excluding bob's known failures passed 3288 tests with three sandbox watcher timeouts. The required bob check passes both build and test on the final commit using its isolated HOME and documented exclusions. Chose the smallest authorized fix: reuse the existing blocking closure.

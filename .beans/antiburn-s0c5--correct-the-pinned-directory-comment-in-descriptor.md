---
# antiburn-s0c5
title: Correct the pinned-directory comment in descriptor_path
status: completed
type: task
priority: low
tags:
    - remote
    - windows
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T03:20:34Z
parent: antiburn-gt1g
---

The comment at `crates/antiburn-remote/src/evidence.rs:370` says that pinned parent directories keep the path. The `pinned` Vec is dropped when `open_relative_file` returns, so nothing pins the path after that. The Analyze claim check limits the effect.

- Keep the directory handles for as long as the path must stay valid, or correct the comment to name what does protect the path. Write the comment in ASD-STE100.
- AC: the comment states what the code does.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Rebased onto feat/all-usage-features. Corrected the comment in `descriptor_path` (evidence.rs): the `pinned` directory handles drop when `open_relative_file` returns, so nothing holds the parent directories after admission. The non-unix path is root path + relative path and can name a different file later. The code is unchanged, so no new test applies. nextest (87 pass), clippy and fmt are clean for antiburn-remote. Bean items are checked. `bob check` passed on the commit.

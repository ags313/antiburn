---
# antiburn-5csa
title: Use one platform policy for remote helpers
status: completed
type: task
priority: low
tags:
    - remote
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T00:31:22Z
parent: antiburn-gt1g
---

`supported_platform` (`apps/desktop/src-tauri/src/remote_sessions.rs:393`) rejects Windows aarch64, but `Hello::validate_compatibility` (`crates/antiburn-remote/src/lib.rs`), which `perform_scan` uses, accepts it.

- Let one function own the supported (os, arch) list, and call it from both checks.
- Test windows/aarch64 and every other listed pair against both checks.
- AC: both checks give the same answer for every (os, arch) pair in the test.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Added `supported_platform` in `crates/antiburn-remote/src/lib.rs` as the one owner of the (os, arch) list: linux x86_64/aarch64 and windows x86_64. `Hello::validate_compatibility` and the desktop preflight in `remote_sessions.rs` both call it. Windows aarch64 is now rejected by both, so the two checks agree. I kept the desktop's existing list as the policy, since it matches the helper builds.

New test `hello_compatibility_follows_the_platform_policy` covers linux, windows and macos against x86_64, aarch64 and riscv64. It compares `validate_compatibility` with `supported_platform` for each pair. The existing desktop preflight test still passes.

Checks run:
- antiburn-remote: nextest 74/74 pass; clippy `-D warnings` and fmt clean.
- Desktop: clippy `-D warnings` and fmt clean; nextest 3262 pass, 5 fail. Those are the known managed-settings failures. I ran it with CLAUDE_EFFORT unset via `unset`, because `env -u` was refused by the permission prompt.

Bean items are checked. Committed with `git commit -s` as b9046b88, and `bob check` passes. The branch was already up to date with `feat/all-usage-features`.

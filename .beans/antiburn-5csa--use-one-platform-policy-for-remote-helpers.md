---
# antiburn-5csa
title: Use one platform policy for remote helpers
status: todo
type: task
priority: low
tags:
    - remote
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`supported_platform` (`apps/desktop/src-tauri/src/remote_sessions.rs:393`) rejects Windows aarch64, but `Hello::validate_compatibility` (`crates/antiburn-remote/src/lib.rs`), which `perform_scan` uses, accepts it.

- Let one function own the supported (os, arch) list, and call it from both checks.
- Test windows/aarch64 and every other listed pair against both checks.
- AC: both checks give the same answer for every (os, arch) pair in the test.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

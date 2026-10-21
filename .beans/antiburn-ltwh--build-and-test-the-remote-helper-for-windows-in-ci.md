---
# antiburn-ltwh
title: Build and test the remote helper for Windows in CI
status: draft
type: task
priority: normal
tags:
    - remote
    - windows
    - ci
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T17:24:11Z
parent: antiburn-gt1g
blocked_by:
    - antiburn-8a0s
---

CI builds only musl targets (`.github/workflows/build-remote-helper.yml`). The `#[cfg(windows)]` code in `crates/antiburn-remote` (path opening without links or junctions, `read_prefix`, database identity) is never compiled or tested in CI, which hid the bundle-copy bug.

- Add a `windows-latest` job that runs clippy and `cargo nextest run` for `crates/antiburn-remote`.
- Say in `docs/remote-sessions.md` which Windows behaviour CI tests.
- AC: a change that breaks Windows-only helper code fails CI.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

---
# antiburn-xw9j
title: Bound a remote listing with one deadline
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

`collector::list` (`crates/antiburn-remote/src/collector.rs:546`) runs the base enumeration (40 s budget) and then each optional store agent in turn, each with its own 20 s `DISCOVERY_TIMEOUT` (`stores.rs:21,77`). This work is outside the request budget. The desktop kills a List request at 60 s (`transport.rs:179`), which fails the whole scan, Claude Code and Codex included.

- Give all of `list` one deadline below the desktop limit. An agent that runs out of time is reported as truncated; the listing does not fail.
- Test with a slow fake store agent: the listing returns within the deadline and holds the other agents' sessions.
- AC: a slow optional agent cannot make the listing fail.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

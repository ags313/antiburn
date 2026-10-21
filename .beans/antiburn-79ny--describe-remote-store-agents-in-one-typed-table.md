---
# antiburn-79ny
title: Describe remote store agents in one typed table
status: todo
type: task
priority: normal
tags:
    - remote
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

Per-agent string dispatch for remote store agents appears at about 12 sites: `crates/antiburn-remote/src/lib.rs:26-30` (`BASE_AGENTS`, `OPTIONAL_AGENTS`, and a hand-copied `SUPPORTED_AGENTS`), `stores.rs:61,105,151,219,243`, `export.rs:77,161,188-200,257`, and `collector.rs:544`.

- Add one `RemoteStore` table keyed by `AgentKind` in `antiburn-remote`: primary path for a session id, companions, `Db | File` source, and a base flag. Derive the agent lists from it.
- Do the helper side only; the desktop cache is a separate bean.
- AC: adding an agent changes only the table and that agent's reader, and the existing behaviour tests pass.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

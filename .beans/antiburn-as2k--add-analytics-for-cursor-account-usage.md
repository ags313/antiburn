---
# antiburn-as2k
title: Add analytics for Cursor account usage
status: todo
type: task
priority: low
tags:
    - cursor
    - analytics
    - size-xs
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
parent: antiburn-xupl
blocked_by:
    - antiburn-mov6
---

`AGENTS.md` asks for analytics on new features; follow `docs/analytics.md` and `docs/analytics-measurement.md`.

- Record the opt-in change and each fetch outcome (ok, signed out, expired, failed), with event counts in coarse buckets and no ids, models, or costs.
- Add the events to the event catalog and its tests.
- AC: the events pass the analytics catalog tests and the review contract.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

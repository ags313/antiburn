---
# antiburn-2ol7
title: Count Cursor account usage in Overview
status: todo
type: feature
priority: normal
tags:
    - cursor
    - usage
    - overview
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
parent: antiburn-xupl
blocked_by:
    - antiburn-uqgk
    - antiburn-mov6
---

Overview totals and the daily chart read `USAGE_EVIDENCE_SQL` through `provider_usage::summarize`; Cursor sessions add nothing there today.

- Feed attributed and account-level Cursor events into the usage rows: tokens on the event's day, recorded cost from Cursor's `totalCents`, under the Cursor agent and provider.
- Count events of remote-matched sessions only where remote sessions count today (`include_remote`); count account-level events in Overview only.
- Test: Overview tokens and cost per day for local, remote, and account-level events; no double count with turn rows.
- AC: with the opt-in on, Overview shows Cursor tokens and cost per day.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

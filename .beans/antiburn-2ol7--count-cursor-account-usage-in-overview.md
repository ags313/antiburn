---
# antiburn-2ol7
title: Count Cursor account usage in Overview
status: completed
type: feature
priority: high
tags:
    - cursor
    - usage
    - overview
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-22T00:18:11Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Cursor account usage now contributes tokens and recorded totalCents cost to Overview totals and daily charts under the Cursor agent and provider. Matched sessions use account events instead of transcript rows, SSH matches require include_remote, and unmatched account events appear only in Overview without adding session counts. The existing opt-in remains the gate.

Added passing synthetic regression tests for local, WSL, remote, and account-level events, daily timezone boundaries, inactive sessions, opt-out, zero/unknown cost, lookback limits, and transcript deduplication. Updated session/check coverage and the analytics coverage description.

Rebased onto feat/all-usage-features (already current). All bean acceptance boxes are checked. Signed commits: 4153a848 and 57d5eecc. No push.

Validation: frontend lint, type-check, 3239 tests, format, design drift, Rust fmt/clippy with -D warnings, and 23 coverage/source contract tests passed. The direct backend run encountered the five documented baseline failures and three sandbox watcher timeouts; bob's full isolated build and test checks passed, including a final bob check on 57d5eecc.

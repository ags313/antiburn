---
# antiburn-r8nz
title: Store Cursor usage events
status: completed
type: task
priority: high
tags:
    - cursor
    - usage
    - store
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T18:08:53Z
parent: antiburn-xupl
blocked_by:
    - antiburn-lmfr
---

Fetched events need a local table, so `get_provider_usage` can read them without a network call.

- Add a `cursor_usage_event` table: `usage_event_id` primary key, `ts_ms`, `model`, `kind`, input, output, and cache-read tokens, recorded cost micros, `conversation_id`, `headless`, `fetched_at`.
- Create it idempotently outside the numbered migrations, as the turn cost column bean decides.
- Insert with insert-or-ignore on `usage_event_id`; prune by the session-retention setting.
- Test: the same event twice gives one row; pruning removes old rows.
- AC: each Cursor usage event is stored once and pruned with session data.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Fixed the landing failure: lib.rs exported two different types named CursorUsageEvent after the parser bean landed. Renamed the stored record to StoredCursorUsageEvent and updated its exports, API signatures, and tests. Kept the parser's CursorUsageEvent export. No lint suppressions or cfg workarounds added.

The requested cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked --features analytics -- -D warnings passes. Formatting and all five Cursor storage tests pass. Final bob check passed build and test, including shell clippy (analytics), on 70aaa338.

Squashed the task into one DCO-signed commit: 70aaa338 Store Cursor usage events with session retention. All bean acceptance boxes remain checked. Working tree is clean. Ready for another landing attempt; nothing pushed.

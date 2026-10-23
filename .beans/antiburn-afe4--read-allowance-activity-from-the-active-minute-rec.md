---
# antiburn-afe4
title: Read allowance activity from the active-minute record
status: completed
type: task
priority: high
tags:
    - overview
    - performance
    - size-xs
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T01:09:41Z
parent: antiburn-urwn
blocked_by:
    - antiburn-7g5n
---

With the active-minute table in place, `attributed_turn_minutes` can read it instead of scanning turn rows.

- Point `attributed_turn_minutes` at the active-minute table; keep its account resolution unchanged.
- While the one-time fill has not completed, keep the current turn scan.
- Test: for fixtures, the old and new query give the same `TurnMinutes` for several windows and accounts.
- Measure on an APFS clone of the operator's dev database (`cp -c` of `~/Library/Application Support/ai.antiburn.desktop.debug/antiburn-debug.sqlite3` with its `-wal`), never on the live file. Report `get_allowance_usage` time cold and warm.
- AC: `attributed_turn_minutes` reads `session_active_minute` after the fill completes, retains the turn scan until then, and the old/new query equivalence tests pass. End-to-end allowance timing is measured; optimizing dollar attribution belongs to a separate bean.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

- Read allowance activity from `session_active_minute` after the fill, with
  the existing turn-scan fallback and unchanged account resolution.
- Verify equivalent `TurnMinutes` across windows and accounts, including
  partial fills and exact timestamp boundaries.
- Measured allowance cold: 143.97 s -> 19.25 s; warm: 61.98 s -> 29.28 s.
- Measured active-minute read: 29 ms; remaining `quota_turn_input` read:
  21.0 s. A separate bean covers dollar-attribution optimization.
- Remove the ignored benchmark's end-to-end one-second assertion. Retain
  result-equivalence checks and cold/warm timing output.

## Implementation and measurements

The reader uses active-minute records after the fill completes. Until then it
uses the existing turn scan. Partial-minute boundaries retain exact timestamp
filtering, and negative-epoch requests retain the scan because the minute table
contains only positive timestamps. Account resolution is unchanged. Fixtures
compare all three `TurnMinutes` maps and resolved accounts over eight windows,
including a partial fill.

Measurements use an APFS clone of the newest stopped-app debug backup, including
its WAL, with 12,363,556 turn rows and 10,645 filled active-minute rows. No live
database is opened. Cold means the first command read on a new connection;
the operating-system cache is not purged.

- Original scan: first run cold 143.97 s, warm 61.98 s; repeat cold 54.82 s,
  warm 64.03 s.
- Active-minute reader: cold connection 19.25 s, warm 29.28 s. The allowance
  results agree with the scan at fixed `now`, allowing relative numeric
  roundoff of 1e-10 from unordered floating-point sums. Integer fields,
  structure, labels, and ordering must match.
- Isolated active-minute attribution: 29.03 ms. The separate, unchanged
  `quota_turn_input` dollar-attribution query takes 21.01 s. Its optimization
  belongs to a separate bean, as agreed by the operator.

The ignored `allowance_active_minutes_clone_benchmark` reproduces the equality
and timing checks without an end-to-end one-second assertion. The ignored
`allowance_clone_read_profile` isolates the two reads on the filled clone.

Validation: the focused equivalence test, `cargo fmt --check`, and clippy with
`-D warnings` pass. The direct full suite runs 3,313 tests: 3,305 pass, the five
known configuration tests fail, and three filesystem-watcher tests time out in
the sandbox. The external `bob check` build and test both pass, with its
documented baseline exclusions. The temporary database clone is deleted.

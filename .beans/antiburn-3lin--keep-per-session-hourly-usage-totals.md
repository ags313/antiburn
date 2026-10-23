---
# antiburn-3lin
title: Keep per-session hourly usage totals
status: completed
type: task
priority: normal
tags:
    - overview
    - performance
    - store
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T00:59:10Z
parent: antiburn-urwn
blocked_by:
    - antiburn-7g5n
---

Overview's daily totals and chart read `USAGE_EVIDENCE_SQL` (`store/usage_evidence.rs`), whose `turn_days` subquery groups each session's published turn rows by local day, model, and speed on every read (191 ms alone today, but it grows with turn rows).

- Add a table of per-session totals keyed by (session, UTC hour start, model, speed): input, output, cache-read, cache-write, and one-hour cache-write tokens, recorded cost micros, the count of rows with a recorded cost, and the row count. Use the same row filter as `turn_days`.
- Maintain it at the same publish and delete points as the active-minute record, and fill it once for existing sessions the same way.
- Fork-only schema changes go in as idempotent steps after the numbered migrations, checked with `pragma_table_info` / `sqlite_master` (as `antiburn-lmfr` did), never as a numbered migration.
- Test with fixtures: after every publish and delete path, the hourly totals sum to the `turn_days` totals.
- AC: hourly totals always equal the turn-row totals they summarize.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Added `session_hourly_usage`, keyed by (session, UTC hour start, model, speed). It holds input, output, cache-read, cache-write and 1h cache-write tokens, recorded cost micros, recorded rows and row count. The new module is `store/hourly_usage.rs`.

- Row filter: it is the `turn_days` filter, with the same published fence, assistant role, non-null model and non-zero token rule. The table keeps ssh rows, because `turn_days` does too.
- Schema: it is created idempotently after the numbered migrations, checked with `sqlite_master`. There is no numbered migration.
- Publish: `publish_projections` rebuilds the session's rows next to the active-minute rebuild.
- Deletes: `delete_session_in` (which retention pruning also uses) and `clear_local_session_data` delete the rows.
- Backfill: `backfill_hourly_usage` runs once at startup after the active-minute fill, in batches of 25 sessions. It records `internal:hourlyUsageBackfillV1`.
- Tests: three store tests compare the table with an independent turn-row scan. They cover first publish, republish, resumed append, session delete, retention pruning, full reset and backfill. A fourth test covers the schema on a fresh DB, an upstream v80 DB and a DB that already has the table.
- Checks: the full nextest run has only the 5 known managed-settings failures, and clippy `-D warnings` and `cargo fmt --check` are clean. `bob check` passed.

Two choices I made:
- A `NULL` speed is stored as `''`, because a primary-key column cannot be `NULL`.
- Rows with no positive timestamp go to `hour_epoch = -1`. Real hour starts are multiples of 3600, so the value cannot collide. `turn_days` gives these rows a `NULL` day.

I did not switch `USAGE_EVIDENCE_SQL` to read the table. The task asked only for the record. The day grouping uses the reader's UTC offset, so the reader change is a separate step.

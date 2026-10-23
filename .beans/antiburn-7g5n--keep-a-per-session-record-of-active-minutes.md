---
# antiburn-7g5n
title: Keep a per-session record of active minutes
status: completed
type: task
priority: high
tags:
    - overview
    - performance
    - store
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T00:21:08Z
parent: antiburn-urwn
---

`Store::attributed_turn_minutes` (`store/provider_limit.rs:651`) needs, for a time window, the distinct (session, minute) pairs with a turn. Today it scans raw turn rows on every Overview read.

- Add a table of (environment_key, agent, session_id, minute_epoch) for each session's published turn rows (`ts_ms > 0`), with an index on minute_epoch. Apply the same row filter as the current query, including its fence rule.
- Rebuild one session's rows in the transaction that publishes its turn rows (`publish_turn_rows`), and delete them wherever that session's turn rows are deleted (session removal, the full reset, retention pruning).
- Fork-only schema changes go in as idempotent steps after the numbered migrations, checked with `pragma_table_info` / `sqlite_master` (as `antiburn-lmfr` did), never as a numbered migration.
- Fill the table for existing sessions once, in the background after startup, in batches that keep the writer responsive; record completion in `setting`.
- Test with fixtures: after publish, republish at a new fence, resumed append, session deletion, and pruning, the table equals the current `SELECT DISTINCT ... (ts_ms / 60000) * 60` result.
- AC: the active-minute table always equals what the current turn scan returns, after every publish and deletion path.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Added `session_active_minute` (environment_key, agent, session_id, minute_epoch), with an index on minute_epoch and a cascade delete from `session`. It is created idempotently after the numbered migrations (checked via `sqlite_master`), not as a numbered migration.

- Publish: `publish_projections` rebuilds the session's rows in the same transaction, after superseded fences are dropped. The row filter matches the current scan: `claim_fence = published_fence`, `ts_ms > 0`, and not `ssh:`.
- Deletes: `delete_session_in` (which retention pruning also uses) and `clear_local_session_data` delete the rows explicitly.
- Backfill: it starts in `spawn_blocking` after startup and runs in batches of 25 sessions, each in its own transaction with a 20 ms pause between batches. It records `internal:activeMinutesBackfillV1 = done` in `setting`. Each rebuild is idempotent, so a race with a publish or a restart is safe.
- Tests: four store tests check the table against the scan. They cover first publish, republish at a new fence, resumed append, dropped source, session delete, ssh exclusion, retention pruning, full reset, and backfill. A fourth-state migration test covers a fresh DB, an upstream v80 DB, and a DB that already has the table.
- Checks: the full desktop nextest run has only the 5 known managed-settings failures. clippy `-D warnings` and `cargo fmt --check` are clean, and `bob check` passes.

I did not change `attributed_turn_minutes` to read the new table. The task only asked for the record. Switching the reader is a separate step, because minute buckets and the current millisecond window edges differ at the boundaries.

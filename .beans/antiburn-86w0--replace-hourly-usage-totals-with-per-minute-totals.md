---
# antiburn-86w0
title: Replace hourly usage totals with per-minute totals
status: completed
type: task
priority: high
tags:
    - overview
    - performance
    - store
    - size-s
created_at: 2026-10-23T00:59:59Z
updated_at: 2026-10-23T01:27:07Z
parent: antiburn-urwn
---

Why per minute: `antiburn-afe4` measured (clone of 2026-10-23, 12.36 M turn rows) that `session_active_minute` holds only 10,645 rows, because turns cluster into few minutes. Per-minute totals therefore stay small, keep minute precision for limit periods, and give exact local days for any UTC offset. After afe4 the allowance command still spends 21.0 s in `read_quota_turn_input` (`store/provider_limit.rs:2197`): `ATTRIBUTED_TURN_BUCKET_SQL` (`:369`) groups published rows by session, model, speed, and 15-minute bucket, and builds `timed_tokens`, a JSON array with every turn's timestamp and tokens. `price_timed_tokens` (`:2102`) prices each turn linearly per token class (recorded cost when present, else catalog rates), so per-minute sums give the same dollars. Hourly totals (`session_hourly_usage`, landed by `antiburn-3lin`) cannot serve this read, because limit periods need minute precision.

- Change the landed `session_hourly_usage` (from `antiburn-3lin`) into per-minute totals keyed by (environment_key, agent, session_id, minute_epoch, model, speed) for each session's published turn rows with `ts_ms > 0`, with an index on minute_epoch. Reuse 3lin's publish hooks, deletion points, schema step, backfill, and fixture tests.
- Columns: the row count; input, output, cache-read, cache-write, and one-hour cache-write token sums over all rows; the same five sums over only the rows without a recorded cost (they are priced from catalog rates); the recorded cost sum; and the count of rows with a recorded cost. These serve both `turn_days` (Overview) and `price_timed_tokens` (quota dollars).
- Check the row filters of `turn_days` (`store/usage_evidence.rs`) and `ATTRIBUTED_TURN_BUCKET_SQL` (for example its token-bearing filter) and store what both need; keep read-time filters (such as `ssh:` environments) at read time.
- Drop `session_hourly_usage` and its index idempotently in the same schema step, and fill the new table once for existing sessions in the background, recorded in `setting` under a new key.
- Fork-only schema changes go in as idempotent steps after the numbered migrations, checked with `pragma_table_info` / `sqlite_master` (as `antiburn-lmfr`, `antiburn-7g5n`, and `antiburn-3lin` did), never as a numbered migration.
- Test with fixtures: after publish, republish at a new fence, resumed append, session deletion, and pruning, the table sums equal the turn-row sums per session, minute, model, and speed, including mixed recorded and unrecorded rows; a database with `session_hourly_usage` ends without it.
- AC: the per-minute totals always equal the turn-row sums they summarize, and `session_hourly_usage` is gone.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Replaced `session_hourly_usage` with `session_minute_usage` (module renamed to `store/minute_usage.rs`).

- Key: (environment_key, agent, session_id, minute_epoch, model, speed), with an index on minute_epoch. `minute_epoch` is in seconds.
- Columns: row_count, five all-row token sums, five `unrecorded_*` token sums (rows without a recorded cost), recorded_cost_micros, recorded_rows.
- Rows: only `ts_ms > 0`. The filter is the `turn_days` one (published fence, assistant, non-null model, non-zero tokens). ssh rows stay in the table, as in `turn_days`; `ssh:` is for the readers to filter.
- Schema: the same idempotent step creates the new table and drops the old table and index. No numbered migration.
- Backfill: runs in the background from the same hook, recorded under `internal:minuteUsageBackfillV1`. Publish, delete, retention and reset hooks are reused.
- Tests: the 3lin fixtures now compare per-minute sums, including mixed recorded and unrecorded rows in one minute. The schema test covers fresh, v80 and current databases. A new test checks that an old hourly table is dropped.
- Checks: nextest, clippy `-D warnings` and fmt pass, and `bob check` passed.

Limit to know for the reader bean (86bu): `ATTRIBUTED_TURN_BUCKET_SQL` has no role or model filter. The table keeps the `turn_days` filter, because `model` is a key column and `turn_days` needs the filter. Rows with a NULL model or a non-assistant role are not in the table.

---
# antiburn-86bu
title: Read quota dollar attribution from per-minute totals
status: completed
type: task
priority: high
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-23T00:59:59Z
updated_at: 2026-10-23T03:40:00Z
parent: antiburn-urwn
blocked_by:
    - antiburn-86w0
---

Why per minute: `antiburn-afe4` measured (clone of 2026-10-23, 12.36 M turn rows) that `session_active_minute` holds only 10,645 rows, because turns cluster into few minutes. Per-minute totals therefore stay small, keep minute precision for limit periods, and give exact local days for any UTC offset. After afe4 the allowance command still spends 21.0 s in `read_quota_turn_input` (`store/provider_limit.rs:2197`): `ATTRIBUTED_TURN_BUCKET_SQL` (`:369`) groups published rows by session, model, speed, and 15-minute bucket, and builds `timed_tokens`, a JSON array with every turn's timestamp and tokens. `price_timed_tokens` (`:2102`) prices each turn linearly per token class (recorded cost when present, else catalog rates), so per-minute sums give the same dollars. Hourly totals (`session_hourly_usage`, landed by `antiburn-3lin`) cannot serve this read, because limit periods need minute precision.

- Read `QuotaTurnInput` rows from the per-minute totals instead of `ATTRIBUTED_TURN_BUCKET_SQL`'s per-turn JSON: each minute row gives a timestamp at the minute start, recorded cost, and the unrecorded token sums for catalog pricing.
- Check how `BucketedSessionDollars::in_period` and the meter allocation use turn timestamps. If a limit period can start or reset inside a minute, keep the turn rows only for minutes that contain such a boundary, or show in a test that boundaries are whole minutes.
- While the per-minute fill has not completed, keep the current query.
- Test: for fixtures, including a period boundary inside a minute, the old and new reads give the same account dollars per period and lane.
- Measure under agent rule 13: an APFS clone (`/bin/cp -c -R`) of the newest stopped-app backup `ai.antiburn.desktop.debug-bob-backup-*` in `$TMPDIR/antiburn-bench-<bean>`, deleted afterwards; never the live folder; report timings and counts only. Report `get_allowance_usage` cold and warm.
- AC: allowance dollars are unchanged, and `get_allowance_usage` takes under 2 s warm on the clone.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

- `read_quota_turn_input` reads `session_minute_usage` once the minute fill is done and `from_epoch >= 0`; otherwise it keeps the turn-row query. New module `store/provider_limit/turn_groups.rs`.
- Minutes that a range edge or a resolved period boundary cuts (via `resolve_periods` per lane) come from turn rows, so period dollars are exact.
- Limit: a meter reading or factor point inside a minute sees that minute's spend at the minute start. The clone benchmark showed the whole allowance summary equal to the turn-row read.
- `BucketedSessionDollars::turn_count` after `in_period` counts usage entries (minutes), not turns. No product code reads it.
- `timed_tokens` entries now hold unrecorded tokens plus the recorded-cost sum; a minute can mix both.
- Tests: `minute_totals_give_the_same_period_dollars_as_the_turn_rows` (also partial fill) and `a_period_boundary_inside_a_minute_keeps_that_minute_s_turn_rows`; both fail when the period cut is disabled.
- Clone measurement (12.5 M turn rows, release build): turn-row read cold 91.5 s, warm 6.85 s; minute fill 2.5 s (26,909 rows); minute-total read cold connection 124 ms, warm 125 ms. Clone deleted.

## Rework: exact meter allocation

Review found that minute-start timestamps changed the meter-segment allocation and factor lookups. Fixed:

- Minutes cut by a range edge, a period boundary, an unaligned factor point, or a meter-segment-closing reading now come from turn rows, summed per session, model, speed, and exact `ts_ms` (`read_timed_groups`). A reading at a minute start cuts its minute too.
- Segment-closing readings come from `segment_closing_readings` in `share.rs`, which `share_period` now uses itself, so the two cannot drift. Other readings divide no usage and do not cut.
- Tests `a_reading_inside_a_minute_splits_measured_and_tail_spend_as_before`, `a_reading_at_a_minute_start_separates_its_own_instant_from_later_turns`, and `a_factor_change_inside_a_minute_prices_the_tail_as_before` compare old and new bucket percent, unexplained segments, and dollars. The reading and factor-point tests fail when their cut is disabled.
- The earlier limit note above no longer applies.
- Clone (release, 12.5 M turn rows): turn-row read warm 6.5 s; minute fill 2.2 s; `get_allowance_usage` with minute totals cold connection 446 ms, warm 454 ms; full allowance summary equal to the turn-row read. Cutting every reading instead took 5.5 s warm. Clone deleted.

## Rework: inclusive `now` cutoff

`in_period` drops turns after `now` inclusive. `quota_turn_input_until(from, to, now)` now cuts the minute that holds `now`, so that minute keeps exact timestamps. `get_allowance_usage`, `quota_usage`, and the per-session quota path pass `now`; closed-window analytics keep `quota_turn_input` (no cutoff). Test `the_now_cutoff_inside_a_minute_clips_the_same_turns_as_the_turn_rows` covers now=119 with a turn at 119500 and to=120, and fails without the cut. Clone: minute totals cold 398 ms, warm 398 ms, allowance equal to the turn-row read. Clone deleted.

## Rework: turns of another role

`session_minute_usage` keeps assistant rows only, but the old read counted every published turn with billable tokens. The minute read now adds a bounded supplemental read of published non-assistant turns with positive tokens (same predicate as `ATTRIBUTED_TURN_BUCKET_SQL`), in the uncut whole minutes; cut minutes already read turn rows of every role, so nothing counts twice. The read uses the partial index `turn_usage_non_assistant_timestamp`, created idempotently at the start of `backfill_minute_usage` (no numbered migration). `backfill_complete` also requires the index, so a database without it keeps the turn-row read until the background step builds it. `role` is NOT NULL, so no NULL roles exist. Tests: `turns_of_another_role_count_once_in_cut_and_uncut_minutes` (fails without the supplemental read) and `the_other_role_read_uses_the_partial_index` (EXPLAIN QUERY PLAN; the real query also forces the index with INDEXED BY). Clone: 0 such rows; index build plus fill 4.4 s once; allowance warm 377 ms, equal to the turn-row read. Clone deleted.

---
# antiburn-86bu
title: Read quota dollar attribution from per-minute totals
status: todo
type: task
priority: high
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-23T00:59:59Z
updated_at: 2026-10-23T00:59:59Z
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

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

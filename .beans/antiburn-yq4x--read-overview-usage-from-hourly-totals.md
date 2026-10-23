---
# antiburn-yq4x
title: Read Overview usage from per-minute totals
status: todo
type: task
priority: normal
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T00:59:59Z
parent: antiburn-urwn
blocked_by:
    - antiburn-86w0
---

With per-minute totals in place, Overview can sum small rows instead of grouping turn rows. `USAGE_EVIDENCE_SQL` (`store/usage_evidence.rs`) groups each session's published rows by local day in its `turn_days` subquery (191 ms alone on 2026-10-23, growing with turn rows).

- Replace the `turn_days` subquery with a read of the per-minute totals, grouped to the reader's local day by the UTC offset. Minute rows are exact for every offset, including +05:30 and +05:45.
- Keep the recorded-cost rule: a model total keeps the recorded cost only when every row has one (use the recorded row count).
- While the per-minute fill has not completed, keep the current query.
- Test: Overview totals and the daily chart match the current query for offsets 0, +60, -300, +330, and +345 minutes, including recorded costs and remote sessions.
- Measure under agent rule 13: an APFS clone (`/bin/cp -c -R`) of the newest stopped-app backup `ai.antiburn.desktop.debug-bob-backup-*` in `$TMPDIR/antiburn-bench-<bean>`, deleted afterwards; never the live folder; report timings and counts only. Report `get_provider_usage` cold and warm.
- AC: Overview usage gives the same figures as before and reads faster on the clone.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

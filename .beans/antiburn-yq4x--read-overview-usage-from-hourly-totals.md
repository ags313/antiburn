---
# antiburn-yq4x
title: Read Overview usage from hourly totals
status: todo
type: task
priority: normal
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-22T23:50:10Z
parent: antiburn-urwn
blocked_by:
    - antiburn-3lin
---

With hourly totals in place, Overview can sum small rows instead of grouping turn rows.

- Replace the `turn_days` subquery in `USAGE_EVIDENCE_SQL` with a read of the hourly totals, shifted to the reader's local day by the UTC offset.
- For an offset that is not a whole hour (for example +05:30 or +05:45), keep the current turn-row query, so day edges stay exact.
- While the one-time fill has not completed, keep the current query.
- Test: Overview totals and the daily chart match the current query for offsets 0, +60, -300, and +330 minutes, including recorded costs and remote sessions.
- Measure on an APFS clone of the operator's dev database (`cp -c` of `~/Library/Application Support/ai.antiburn.desktop.debug/antiburn-debug.sqlite3` with its `-wal`), never on the live file. Report `get_provider_usage` time cold and warm.
- AC: Overview usage gives the same figures as before and reads faster on the clone.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

---
# antiburn-3lin
title: Keep per-session hourly usage totals
status: todo
type: task
priority: normal
tags:
    - overview
    - performance
    - store
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-22T23:50:10Z
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

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

---
# antiburn-7g5n
title: Keep a per-session record of active minutes
status: todo
type: task
priority: high
tags:
    - overview
    - performance
    - store
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-22T23:50:10Z
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

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

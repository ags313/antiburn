---
# antiburn-urwn
title: Make Overview reads fast with per-session summaries
status: completed
type: epic
priority: high
tags:
    - overview
    - performance
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T04:12:51Z
---

Evidence (2026-10-23, operator's dev database: 12.35 M turn rows for 1,335 sessions, turn table about 3 GB): all Overview reads share one `UiReadStore` connection (`apps/desktop/src-tauri/src/lib.rs:161`, one `Mutex<Connection>`, `store/mod.rs:830`), so they run one at a time. `get_allowance_usage` took 89.7 s after a restart and 21-23 s warm; `get_session_limit_allocations` finished 50 ms after it each time but took 69 ms alone; `get_provider_usage` took 75 s queued and 191 ms alone; `get_live_usage` 75 s queued and 7 ms alone. The slow part of `get_allowance_usage` is `Store::attributed_turn_minutes` (`store/provider_limit.rs:651`), which runs `SELECT DISTINCT environment_key, agent, session_id, (ts_ms / 60000) * 60` over raw turn rows for the window on every read. `session_analysis` already holds per-session model and pricing breakdowns, but it has no time dimension, so per-day usage and per-minute activity still come from raw turn rows.

Plan: give Overview reads their own connections, so one slow read never blocks the others; keep per-session summaries with a time dimension, maintained in the transaction that publishes a session's turn rows; and point the slow reads at them. A session's published rows are those at `session_evidence.published_fence`; they change only when a pass publishes (`publish_turn_rows`, `store/publication.rs:45`) or rows are deleted (session removal `store/session_persistence.rs:555,648`, the full reset at `store/mod.rs:2198`, fence cleanup in `crates/antiburn-local/src/analysis/rows.rs`, retention pruning). Rebuilding one session's summary at those points keeps it exact, with no guess about when a session is finished.

Fork-only schema changes go in as idempotent steps after the numbered migrations, checked with `pragma_table_info` / `sqlite_master` (as `antiburn-lmfr` did), never as a numbered migration.

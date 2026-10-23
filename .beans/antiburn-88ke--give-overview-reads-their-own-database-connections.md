---
# antiburn-88ke
title: Give Overview reads their own database connections
status: completed
type: task
priority: high
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-23T00:50:19Z
parent: antiburn-urwn
---

Evidence (2026-10-23, operator's dev database: 12.35 M turn rows for 1,335 sessions, turn table about 3 GB): all Overview reads share one `UiReadStore` connection (`apps/desktop/src-tauri/src/lib.rs:161`, one `Mutex<Connection>`, `store/mod.rs:830`), so they run one at a time. `get_allowance_usage` took 89.7 s after a restart and 21-23 s warm; `get_session_limit_allocations` finished 50 ms after it each time but took 69 ms alone; `get_provider_usage` took 75 s queued and 191 ms alone; `get_live_usage` 75 s queued and 7 ms alone. The slow part of `get_allowance_usage` is `Store::attributed_turn_minutes` (`store/provider_limit.rs:651`), which runs `SELECT DISTINCT environment_key, agent, session_id, (ts_ms / 60000) * 60` over raw turn rows for the window on every read. `session_analysis` already holds per-session model and pricing breakdowns, but it has no time dimension, so per-day usage and per-minute activity still come from raw turn rows.

- Replace the single `UiReadStore` connection with a small pool of read-only WAL connections (for example 4), or one reader per Overview command, so `get_provider_usage`, `get_live_usage`, `get_allowance_usage`, and `get_session_limit_allocations` read in parallel.
- Keep the existing busy timeout and the fallback when a reader cannot open.
- Test: while one reader is held inside a long read, another Overview read on the pool completes.
- Measure on an APFS clone of the operator's dev database (`cp -c` of `~/Library/Application Support/ai.antiburn.desktop.debug/antiburn-debug.sqlite3` with its `-wal`), never on the live file. Report `overview_read_timing` for each command after a cold start.
- AC: a slow `get_allowance_usage` no longer delays `get_provider_usage` or `get_live_usage`; each reports its own time in `overview_read_timing`.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Validation (2026-10-23)

Fresh APFS clone of the newest `debug-bob-backup` (1,335 sessions, 13.18 M turn rows), fresh process, 4-reader pool. The four reads started together. Each ran the command's blocking body: checkout, store read, and the real `log_overview_read_timing` call, with a tracing subscriber capturing the event (debug level enabled). OS file cache not purged.

| command | `overview_read_timing` elapsed_ms |
|---|---|
| `get_live_usage` | 12 |
| `get_session_limit_allocations` | 176 |
| `get_provider_usage` | 5,133 |
| `get_allowance_usage` | 175,866 |

The first three finished while `get_allowance_usage` was still running. The Tauri `#[tauri::command]` wrappers were not invoked: they take an `AppHandle<Wry>`, which cannot be built headless. For `get_live_usage` this leaves out the in-memory live-summary and detection step; the database part is the settings read. The harness was temporary and is not committed.

## Summary of Changes

Re-measured on a fresh APFS clone of the newest debug-bob-backup. The clone is deleted, the live database was never opened, and no session content was read. It held 1,335 sessions and 13.18 M turn rows. I used a fresh process, a 4-reader pool and all four reads started together, with the OS file cache not purged.

Each read ran its command's blocking body: checkout, store read, and the real `log_overview_read_timing` call. A tracing subscriber captured the `overview_read_timing` event with debug logging enabled. The `elapsed_ms` values were:
- get_live_usage: 12
- get_session_limit_allocations: 176
- get_provider_usage: 5,133
- get_allowance_usage: 175,866

The first three finished while get_allowance_usage was still running.

Limit: I did not invoke the `#[tauri::command]` wrappers themselves. They take an `AppHandle<Wry>`, which cannot be built headless. For get_live_usage this leaves out the in-memory live-summary and detection step. The database part of that command is the settings read, which was measured.

The harness was temporary and is not committed. The bean's Validation section is updated with these numbers and the limit. `bob check` passes.

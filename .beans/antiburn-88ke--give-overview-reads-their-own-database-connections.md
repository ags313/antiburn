---
# antiburn-88ke
title: Give Overview reads their own database connections
status: todo
type: task
priority: high
tags:
    - overview
    - performance
    - size-s
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-22T23:50:10Z
parent: antiburn-urwn
---

Evidence (2026-10-23, operator's dev database: 12.35 M turn rows for 1,335 sessions, turn table about 3 GB): all Overview reads share one `UiReadStore` connection (`apps/desktop/src-tauri/src/lib.rs:161`, one `Mutex<Connection>`, `store/mod.rs:830`), so they run one at a time. `get_allowance_usage` took 89.7 s after a restart and 21-23 s warm; `get_session_limit_allocations` finished 50 ms after it each time but took 69 ms alone; `get_provider_usage` took 75 s queued and 191 ms alone; `get_live_usage` 75 s queued and 7 ms alone. The slow part of `get_allowance_usage` is `Store::attributed_turn_minutes` (`store/provider_limit.rs:651`), which runs `SELECT DISTINCT environment_key, agent, session_id, (ts_ms / 60000) * 60` over raw turn rows for the window on every read. `session_analysis` already holds per-session model and pricing breakdowns, but it has no time dimension, so per-day usage and per-minute activity still come from raw turn rows.

- Replace the single `UiReadStore` connection with a small pool of read-only WAL connections (for example 4), or one reader per Overview command, so `get_provider_usage`, `get_live_usage`, `get_allowance_usage`, and `get_session_limit_allocations` read in parallel.
- Keep the existing busy timeout and the fallback when a reader cannot open.
- Test: while one reader is held inside a long read, another Overview read on the pool completes.
- Measure on an APFS clone of the operator's dev database (`cp -c` of `~/Library/Application Support/ai.antiburn.desktop.debug/antiburn-debug.sqlite3` with its `-wal`), never on the live file. Report `overview_read_timing` for each command after a cold start.
- AC: a slow `get_allowance_usage` no longer delays `get_provider_usage` or `get_live_usage`; each reports its own time in `overview_read_timing`.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

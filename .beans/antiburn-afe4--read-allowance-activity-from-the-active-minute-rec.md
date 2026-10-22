---
# antiburn-afe4
title: Read allowance activity from the active-minute record
status: todo
type: task
priority: high
tags:
    - overview
    - performance
    - size-xs
created_at: 2026-10-22T23:50:10Z
updated_at: 2026-10-22T23:50:10Z
parent: antiburn-urwn
blocked_by:
    - antiburn-7g5n
---

With the active-minute table in place, `attributed_turn_minutes` can read it instead of scanning turn rows.

- Point `attributed_turn_minutes` at the active-minute table; keep its account resolution unchanged.
- While the one-time fill has not completed, keep the current turn scan.
- Test: for fixtures, the old and new query give the same `TurnMinutes` for several windows and accounts.
- Measure on an APFS clone of the operator's dev database (`cp -c` of `~/Library/Application Support/ai.antiburn.desktop.debug/antiburn-debug.sqlite3` with its `-wal`), never on the live file. Report `get_allowance_usage` time cold and warm.
- AC: `get_allowance_usage` gives the same result and takes under 1 s warm on the clone.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

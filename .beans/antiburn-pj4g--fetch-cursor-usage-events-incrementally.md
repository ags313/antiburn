---
# antiburn-pj4g
title: Fetch Cursor usage events incrementally
status: todo
type: task
priority: high
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T18:08:53Z
parent: antiburn-xupl
blocked_by:
    - antiburn-uadl
    - antiburn-lrny
---

The spike called `POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetFilteredUsageEvents` (Connect JSON, `Connect-Protocol-Version: 1`, `Authorization: Bearer <token>`) with `{startDate, endDate, page, pageSize: 100}` (dates as ms strings). 30 days took 8 pages; the account had 986 events back to 2026-03-27.

- Add `sources/cursor_fetch.rs` on the shared client in `sources/http.rs` and the cooldown in `sources/cooldown.rs`.
- Fetch from the newest stored event minus one day of overlap to now; the first run fetches back to the session-retention window. Cap pages per run.
- A 401 or 403 marks the login expired and stops; it does not retry in a loop. Any other failure leaves stored events unchanged.
- Test against a fake HTTP server: paging, overlap, 401, a malformed page.
- Never print, log, or store the Cursor token; send it only to `api2.cursor.sh`; never use `cursorAuth/refreshToken` (a refresh can sign the Cursor IDE out). Test fixtures are synthetic: no real usage data, ids, or tokens.
- AC: repeated runs fetch only new events, and failures keep the stored events.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

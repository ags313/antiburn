---
# antiburn-mov6
title: Add an opt-in for Cursor account usage
status: todo
type: feature
priority: high
tags:
    - cursor
    - usage
    - settings
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T18:08:53Z
parent: antiburn-xupl
blocked_by:
    - antiburn-pj4g
    - antiburn-r8nz
---

Reading the Cursor account sends the local Cursor token to Cursor. The user must turn it on.

- Add a setting (default off) and a control in Settings beside the session sources (`overview/stepSettings/SessionSourcesSections.tsx`). Register it through the Settings adapters so search finds it.
- The control text says what is read (usage events with tokens, cost, model, time), where the token goes (`api2.cursor.sh` only), and that the Cursor IDE must be signed in. Show the login state: signed in, signed out, expired.
- Fetch only while the setting is on: at start, then on a slow interval (for example every 30 minutes) and after Cursor activity, through the cooldown. Turning it off stops fetching and hides Cursor account figures.
- Test the setting round trip, the gate, and the login-state display.
- AC: with the setting off, nothing contacts Cursor; with it on, events arrive without user action.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

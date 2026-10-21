---
# antiburn-mov6
title: Add an opt-in for Cursor account usage
status: completed
type: feature
priority: high
tags:
    - cursor
    - usage
    - settings
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T23:42:41Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Validation

- `bob check` passed build and test on `97cb7982`, including Rust checks with and without analytics, workspace tests, frontend checks, and script tests.
- All 56 focused Cursor-related Rust tests passed. The frontend suite passed 3,237 tests with one existing skip.
- The direct sandboxed desktop nextest run passed 3,276 tests; the five known managed-settings failures and three filesystem-watcher failures remained. The canonical `bob check` test run passed with its configured baseline exclusions and empty test home.
- Consent defaults off. Cursor refresh runs independently of the other providers' Live Usage switch. Opt-out retains stored events under local retention but hides account attribution and prevents further request pages.

## Summary of Changes

Added default-off Cursor account usage consent in searchable Settings beside session sources. The control explains usage fields, api2.cursor.sh-only credential delivery, and the signed-in IDE requirement; it displays signed in, signed out, expired, or unavailable, including server-rejected credentials.

Enabled accounts fetch at startup, every 30 minutes, and after Cursor activity through the shared cooldown. Opt-out blocks credential reads for fetching, stops subsequent pages, and hides account attribution. Safe defaults: this consent is independent of the other providers' Live Usage switch; stored events remain under local retention.

Added synthetic-server and temporary-store tests for persistence, off/on/off gating, pagination cancellation, login states, automatic scheduling, Cursor-only activity, search, and focus without changing consent. All 56 focused Rust tests and 3,237 frontend tests passed. Canonical bob build and test passed on final commit e6ec05aa; an intervening analytics-suite check failure passed on unchanged retry. Direct sandbox nextest also exposed the five known config failures and three watcher failures; the canonical check passed with its configured baseline exclusions.

Rebased onto feat/all-usage-features. All bean acceptance items are checked and committed. DCO-signed commits: 97cb7982 and e6ec05aa. Working tree clean. Nothing pushed.

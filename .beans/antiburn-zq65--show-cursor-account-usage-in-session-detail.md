---
# antiburn-zq65
title: Show Cursor account usage in session detail
status: completed
type: feature
priority: high
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-22T01:04:35Z
parent: antiburn-xupl
blocked_by:
    - antiburn-uqgk
    - antiburn-mov6
---

A Cursor session's detail shows no tokens or cost, because its transcript holds none.

- Show the session's attributed tokens and cost in session detail, labelled as coming from the Cursor account.
- Test the label and the totals for a matched session, and the empty state for an unmatched one.
- AC: a matched Cursor session shows its account tokens and cost.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Rebased onto feat/all-usage-features and resolved the conflict in apps/desktop/src-tauri/src/store/cursor_usage.rs by preserving both the base branch's cursor_usage_evidence method and the session-detail cursor_account_usage_for_session method. Both sets of tests remain intact.

Rebased commits retain DCO sign-offs. Final commit: 4168ebaf. Working tree is clean. bob check passed build and test on the final rebased commit. Nothing was pushed.

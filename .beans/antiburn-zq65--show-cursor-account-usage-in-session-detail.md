---
# antiburn-zq65
title: Show Cursor account usage in session detail
status: todo
type: feature
priority: low
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
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

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

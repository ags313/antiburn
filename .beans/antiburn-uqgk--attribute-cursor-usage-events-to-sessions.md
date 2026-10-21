---
# antiburn-uqgk
title: Attribute Cursor usage events to sessions
status: todo
type: task
priority: normal
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
parent: antiburn-xupl
blocked_by:
    - antiburn-r8nz
---

In the spike, `conversationId` matched a stored Cursor session's id or transcript file stem for 178 of 186 conversations, 120 local and 58 synced from a remote host. The usage belongs to the account, so it can match sessions in any environment.

- Add a query that joins `cursor_usage_event.conversation_id` to Cursor sessions in every environment. One event counts for exactly one session; if a conversation exists in two environments, choose one deterministically.
- Keep unmatched events as account-level Cursor usage.
- Test: a local match, a remote match, an unmatched event, and a conversation present in two environments counted once.
- AC: every stored event counts once: for one session, or as account-level usage.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

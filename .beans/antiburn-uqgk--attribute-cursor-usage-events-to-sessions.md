---
# antiburn-uqgk
title: Attribute Cursor usage events to sessions
status: completed
type: task
priority: high
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T22:24:27Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Added `Store::attributed_cursor_usage_events(start_ms, end_ms)` in `store/cursor_usage.rs`. It joins `conversation_id` to Cursor sessions in every environment, matching on session id or transcript file stem. Each event gets one session or `None` (account-level).

Tie-break when a conversation is in several environments: an own-id match wins over a stem match. After that, the lowest `(environment_key, session_id)` wins.

Test covers a local match, a remote match, an unmatched event, an event with no conversation, and a conversation in two environments counted once.

Checks: fmt and clippy `-D warnings` are clean. Full nextest has only the 5 known managed-settings failures. `bob check` passes. Bean boxes are ticked.

Assumption: the transcript stem is taken from `session.source_label`, which holds the transcript path for file sources.

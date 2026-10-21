---
# antiburn-lrny
title: Parse Cursor usage-event pages strictly
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
---

`GetFilteredUsageEvents` returns `totalUsageEventsCount` and `usageEventsDisplay[]` with `timestamp` (ms as a string), `model`, `kind`, `tokenUsage {inputTokens, outputTokens, cacheReadTokens, totalCents}`, `chargedCents`, `conversationId`, `isHeadless`, and `usageEventId`.

- Add a pure parser (no I/O), in the style of `provider_usage/live/anthropic.rs`: keep only the fields above. A malformed field that is present rejects the page; a missing `tokenUsage` gives zero tokens and no cost; unknown fields are ignored.
- Convert `totalCents` to micros of a dollar for the recorded cost.
- Test with synthetic pages: full, missing usage, malformed number, unknown fields.
- AC: parsed events carry exact tokens and cost, and malformed pages are rejected.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

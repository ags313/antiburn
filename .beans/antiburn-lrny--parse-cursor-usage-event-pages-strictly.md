---
# antiburn-lrny
title: Parse Cursor usage-event pages strictly
status: completed
type: task
priority: normal
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T19:06:30Z
parent: antiburn-xupl
---

`GetFilteredUsageEvents` returns `totalUsageEventsCount` and `usageEventsDisplay[]` with `timestamp` (ms as a string), `model`, `kind`, `tokenUsage {inputTokens, outputTokens, cacheReadTokens, totalCents}`, `chargedCents`, `conversationId`, `isHeadless`, and `usageEventId`.

- Add a pure parser (no I/O), in the style of `provider_usage/live/anthropic.rs`: keep only the fields above. A malformed field that is present rejects the page; a missing `tokenUsage` gives zero tokens and no cost; unknown fields are ignored.
- Convert `totalCents` to micros of a dollar for the recorded cost.
- Test with synthetic pages: full, missing usage, malformed number, unknown fields.
- AC: parsed events carry exact tokens and cost, and malformed pages are rejected.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Addressed the review on antiburn-lrny: removed the const_ dead-code bridge and parse_usage_page_value; kept the parser reachable via a crate-root pub use (same idea as the Store bridge, not a type-equality lint trick). Connect JSON defaults: missing isHeadless→false, chargedCents/totalUsageEventsCount→0, usageEventsDisplay→[], model/kind/conversationId→""; timestamp and usageEventId stay required with a comment. ASD-STE100 comment pass. Added omitted-defaults and wrong-type-for-defaultable-field tests. bob check passed.

---
# antiburn-xupl
title: Show Cursor usage from the Cursor account
status: todo
type: epic
priority: normal
tags:
    - cursor
    - usage
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
---

Cursor sessions show no tokens or cost: no local Cursor file records usage. A spike on 2026-10-21 showed that Cursor's account API, called with the local Cursor IDE login, returns per-request tokens, Cursor's cost value, model, time, and `conversationId`. 98 % of requests matched Cursor sessions already in the index, local and remote.

Decisions:

- Opt-in, default off. The fetch is a separate account-usage source that reuses the live-usage HTTP client and cooldown (`apps/desktop/src-tauri/src/provider_usage/live/sources/{http,cooldown}.rs`). It is not a `LiveUsageSource`: live usage repeats limits only, and the estimate payload (`get_provider_usage`) must stay free of limits.
- Fetched events go into the local store; `get_provider_usage` reads them and still contacts no provider.
- Cursor's `totalCents` is the cost (a recorded cost); catalog prices do not know Cursor models such as `composer-2.5`.
- Each `usageEventId` counts once. Unmatched events stay account-level.
- Fail closed: a private API can change; then show that Cursor account usage is unavailable and keep everything else.

Never print, log, or store the Cursor token; send it only to `api2.cursor.sh`; never use `cursorAuth/refreshToken` (a refresh can sign the Cursor IDE out). Test fixtures are synthetic: no real usage data, ids, or tokens.

Spike: `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

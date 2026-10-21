---
# antiburn-zlpq
title: Make the Overview read pacing match its comments
status: todo
type: task
priority: low
tags:
    - overview
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

In `apps/desktop/src/views/main-window/MainOverviewSession.ts:94,104,292-293,498`, `OVERVIEW_EVENT_INTERVAL_MS` (30 s) plus `readDelay` sits over the 300 ms debounce and the 5 s backlog throttle. The comment "One read every 5 s" is no longer true.

- Remove the throttle that no longer has an effect, or correct the comments to the real cadence; keep the timer tests.
- AC: the comments state the real read cadence, and the timer tests pass.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

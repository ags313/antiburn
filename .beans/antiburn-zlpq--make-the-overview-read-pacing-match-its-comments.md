---
# antiburn-zlpq
title: Make the Overview read pacing match its comments
status: completed
type: task
priority: low
tags:
    - overview
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T02:31:12Z
parent: antiburn-gt1g
---

In `apps/desktop/src/views/main-window/MainOverviewSession.ts:94,104,292-293,498`, `OVERVIEW_EVENT_INTERVAL_MS` (30 s) plus `readDelay` sits over the 300 ms debounce and the 5 s backlog throttle. The comment "One read every 5 s" is no longer true.

- Remove the throttle that no longer has an effect, or correct the comments to the real cadence; keep the timer tests.
- AC: the comments state the real read cadence, and the timer tests pass.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Corrected Overview pacing comments to state the 30-second event interval over the 300 ms debounce and 5-second backlog minimum waits. Preserved runtime behavior and existing timer tests; extended interval coverage to run with and without an active backlog.

Rebased onto feat/all-usage-features (already current). All bean acceptance items are checked and committed in signed commit 03306678, Clarify Overview event read cadence. No Co-authored-by trailer.

Passed desktop lint, type-check, test (3242 passed, 1 skipped, plus 2 runner tests), format, and design drift checks. bob check passed on the last commit. Nothing pushed.

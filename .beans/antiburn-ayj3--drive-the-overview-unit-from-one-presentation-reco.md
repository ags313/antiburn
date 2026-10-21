---
# antiburn-ayj3
title: Drive the Overview unit from one presentation record
status: completed
type: task
priority: low
tags:
    - overview
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T23:11:05Z
parent: antiburn-gt1g
---

`OverviewSpendChart.tsx` and `OverviewSpendTotals.tsx` hold 11 `unit === "cost"` branches (value, labels, axis, two aria-labels, banner, incomplete flag, tooltip, cell). The components are still named `*Spend*` but now show tokens too.

- Add `UNIT_PRESENTATION: Record<LocalUsageUnit, {...}>`, and rename the components to `OverviewUsageChart` and `OverviewUsageTotals`.
- AC: no `unit === "cost"` branches remain, and the tests pass.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Rebased onto feat/all-usage-features (already current). Added a typed UNIT_PRESENTATION record for Overview cost/token values, labels, axes, accessibility labels, banners, completeness, tooltips, and total cells. Renamed the components and tests to OverviewUsageChart and OverviewUsageTotals and updated their callers. No unit comparisons remain in these components.

Preserved existing behavior and added coverage for both unit banners, missing agent breakdowns, and empty days. Desktop lint, type-check, test (3232 passed, 1 skipped), format, and design-drift checks pass. All acceptance items are checked in the bean.

Committed with DCO sign-off as 81884022: refactor(desktop): drive Overview usage from one presentation record. bob check passed on this final commit. No push performed.

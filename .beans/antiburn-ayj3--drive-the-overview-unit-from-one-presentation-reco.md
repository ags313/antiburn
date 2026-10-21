---
# antiburn-ayj3
title: Drive the Overview unit from one presentation record
status: todo
type: task
priority: low
tags:
    - overview
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`OverviewSpendChart.tsx` and `OverviewSpendTotals.tsx` hold 11 `unit === "cost"` branches (value, labels, axis, two aria-labels, banner, incomplete flag, tooltip, cell). The components are still named `*Spend*` but now show tokens too.

- Add `UNIT_PRESENTATION: Record<LocalUsageUnit, {...}>`, and rename the components to `OverviewUsageChart` and `OverviewUsageTotals`.
- AC: no `unit === "cost"` branches remain, and the tests pass.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

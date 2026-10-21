---
# antiburn-4bzc
title: Give Copilot and Amp their own Overview colours
status: completed
type: bug
priority: normal
tags:
    - overview
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T21:37:07Z
parent: antiburn-gt1g
---

`AGENT_STYLES` (`apps/desktop/src/views/main-window/overview/OverviewSpendChart.tsx:28-40`) lists 11 agents, but the agent registry (`apps/desktop/src/lib/presentation/agents.ts`) has 13. `copilot` and `amp-code` draw in the neutral colour, although `apps/desktop/design.md` gives each known agent its own hue.

- Put the literal `fill` and `swatch` classes on the registry entries (literal strings, so Tailwind finds them). Add the missing tokens to `styles/session-analysis-colors.css` and `design.md`.
- Test: every registry agent has a style that is not the neutral one.
- AC: every agent in the registry has its own Overview colour.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

1. Added `agent-copilot` (muted indigo) and `agent-amp-code` (muted crimson) design tokens to `apps/desktop/design.md` and `apps/desktop/src/styles/session-analysis-colors.css`.
2. Updated `AgentInfo` in `apps/desktop/src/lib/presentation/agents.ts` with literal `fill` and `swatch` classes on every registry agent entry, and exported `agentStyle` and `OTHER_AGENT_STYLE`.
3. Updated `apps/desktop/src/views/main-window/overview/OverviewSpendChart.tsx` to import `agentStyle` from `agents.ts` instead of maintaining a drifted local table.
4. Added tests verifying that every registry agent has its own unique Overview colour that is not neutral, and that Copilot and Amp render with their respective fill classes in the Overview chart.
5. Verified clean run of `lint`, `type-check`, `test`, `format`, `check-design-drift.mjs`, and passing `bob check`.

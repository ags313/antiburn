---
# antiburn-wjfk
title: Pace checks refreshes in one place
status: draft
type: task
priority: normal
tags:
    - insights
    - overview
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T17:24:11Z
parent: antiburn-gt1g
---

Checks refreshes are throttled twice: in the worker (`apps/desktop/src-tauri/src/insights_worker.rs:47-50`: `REPORT_IDLE_INTERVAL` 30 s, `reported_at` and `last_report_at`, `import` and `import_drained`) and in the consumer (`overviewProgressStore.ts:634`, `STEADY_CHECKS_INTERVAL_MS` 30 s). Checks can be about 60 s stale.

- Keep the producer pacing as one `ReportPacer` struct with one timestamp and one import state. Remove the consumer timer, or make it read on signal.
- Measure CPU again with the optimized build under agent load; it must not exceed the 2026-10-19 result (about 12-16%).
- AC: Checks are at most 30 s stale, there is one pacing setting, and CPU is not higher.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

---
# antiburn-bg2o
title: Remove the ModelTokens identity wrappers
status: completed
type: task
priority: low
tags:
    - engine
    - refactor
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T23:53:31Z
parent: antiburn-gt1g
---

`provider_usage::add_model_tokens` (`apps/desktop/src-tauri/src/provider_usage/mod.rs:284`) and `tally::add_model_tokens` (`crates/antiburn-local/src/analysis/metrics_sink/tally.rs:282`) only call `.add()`. The `ModelTokens { .. }` construction repeats in `query_model_breakdown`, `query_pricing_breakdown`, and `usage_turn_days`.

- Call `.add()` directly, and add `ModelTokens::from_usage(usage, recorded)`.
- AC: no wrapper functions remain, and the tests pass.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Validation: both raw nextest commands reached documented baseline failures.
The full isolated suites passed through `bob check`, with its configured
baseline exclusions. Both package Clippy checks and formatting checks passed.
The constructor test covers every token field and absent, zero, and maximum
recorded costs; existing aggregation and provider usage tests also pass.

## Summary of Changes

Rebased onto feat/all-usage-features (already current). Removed both ModelTokens identity wrappers and replaced their callers with direct .add() calls. Added ModelTokens::from_usage and used it in the model/pricing queries, usage_turn_days, and tally usage conversion. Added tests for all token fields and absent, zero, and maximum recorded costs.

All four bean acceptance items are checked and committed. Signed commits: e2ed1541 and ecac55f0. Formatting and Clippy with -D warnings pass for both packages. Raw nextest commands reached the documented baseline failures; full isolated suites pass through bob check with its configured baseline exclusions. Final bob check passed build and test on ecac55f0. Working tree is clean. Nothing pushed.

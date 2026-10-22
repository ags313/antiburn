---
# antiburn-d4qt
title: Overview usage survives token sums that overflow
status: completed
type: bug
priority: low
tags:
    - store
    - overview
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T00:04:39Z
parent: antiburn-gt1g
---

`USAGE_EVIDENCE_SQL` (`apps/desktop/src-tauri/src/store/mod.rs:466-471`) adds tokens and `recorded_cost_micros` with SQL `SUM()`, which raises "integer overflow". One garbage session, local or synced from a remote host, makes the read fail, and Overview shows "Local usage is unavailable". `crates/antiburn-local/src/analysis/vendors/junie.rs:174` adds with `+=`, which panics in debug builds.

- Use `TOTAL()` and convert, or saturate the values when the rows are written. Use `saturating_add` in `junie.rs`.
- Test: one turn with `i64::MAX` tokens plus another turn; the usage read succeeds.
- AC: the Overview usage read succeeds and saturates.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Validation: both overflow regression tests pass. Both workspaces pass Clippy with
`-D warnings` and formatting checks. Direct full-suite runs encounter local Git,
managed-settings, and filesystem-watcher failures; the required `bob check`
passes both build and test with its isolated environment and configured exclusions.

## Summary of Changes

Rebased onto feat/all-usage-features (already up to date). Overview usage sums now use TOTAL with INTEGER casts, saturating token and recorded-cost totals at i64::MAX instead of failing. Junie model token totals use saturating_add within and across events. Added and passed regression tests for both overflow paths.

Both workspaces pass formatting and Clippy with -D warnings. Direct full-suite runs encountered local Git, managed-settings, and watcher failures; the required bob check passed build and test with its isolated environment and configured exclusions, including on final commit c8cd9475. All acceptance checkboxes are checked and committed. Both commits have DCO sign-offs; nothing was pushed.

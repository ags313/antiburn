---
# antiburn-d4qt
title: Overview usage survives token sums that overflow
status: todo
type: bug
priority: low
tags:
    - store
    - overview
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`USAGE_EVIDENCE_SQL` (`apps/desktop/src-tauri/src/store/mod.rs:466-471`) adds tokens and `recorded_cost_micros` with SQL `SUM()`, which raises "integer overflow". One garbage session, local or synced from a remote host, makes the read fail, and Overview shows "Local usage is unavailable". `crates/antiburn-local/src/analysis/vendors/junie.rs:174` adds with `+=`, which panics in debug builds.

- Use `TOTAL()` and convert, or saturate the values when the rows are written. Use `saturating_add` in `junie.rs`.
- Test: one turn with `i64::MAX` tokens plus another turn; the usage read succeeds.
- AC: the Overview usage read succeeds and saturates.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

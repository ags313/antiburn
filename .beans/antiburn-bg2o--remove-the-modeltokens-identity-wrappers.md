---
# antiburn-bg2o
title: Remove the ModelTokens identity wrappers
status: todo
type: task
priority: low
tags:
    - engine
    - refactor
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`provider_usage::add_model_tokens` (`apps/desktop/src-tauri/src/provider_usage/mod.rs:284`) and `tally::add_model_tokens` (`crates/antiburn-local/src/analysis/metrics_sink/tally.rs:282`) only call `.add()`. The `ModelTokens { .. }` construction repeats in `query_model_breakdown`, `query_pricing_breakdown`, and `usage_turn_days`.

- Call `.add()` directly, and add `ModelTokens::from_usage(usage, recorded)`.
- AC: no wrapper functions remain, and the tests pass.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

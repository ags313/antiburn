---
# antiburn-awfr
title: Use OpenCode recorded cost in per-turn and HUD pricing
status: completed
type: feature
priority: low
tags:
    - pricing
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T03:11:53Z
parent: antiburn-gt1g
---

Session cost and Overview use OpenCode's recorded cost for local providers. Per-turn and HUD pricing (`apps/desktop/src-tauri/src/insights_report.rs:1045,1386`) set `recorded_cost_micros: None`, so a local Ollama model shows a catalog price there.

- Read `turn.recorded_cost_micros` into those turn costs.
- Update the "Per-turn cost views ... do not read that column yet" text in `docs/session-coverage.md`.
- AC: a local OpenCode turn shows its recorded cost (normally 0) in per-turn and HUD views.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Review P1 fixed: the HUD spend rate now uses the recorded cost. `bob check` passed on commit 3cfcd82c, on top of 5befc515.

- crates/antiburn-local/src/analysis/modes.rs: `ModeSample` has a new `recorded_cost_micros` field. It is copied from the normalized turn onto the first sample of each turn only, like `usage`.
- apps/desktop/src-tauri/src/hud_token_map.rs: `SpendTally::add` now goes through `ModelTokens::add`, so a model keeps its recorded cost only when every turn in it has one. `rate()` then uses it through `price_breakdown`.
- HUD regression test `a_local_opencode_turn_with_a_catalog_model_name_costs_zero_in_the_hud`: a real OpenCode SQLite fixture with provider `ollama`, model `claude-opus-4-6` and cost 0, run through `normalize_source`, `mode_samples` and `SpendTally`. It asserts $0 spend and priced_share 1.0.
- Two more tests: `a_recorded_cost_replaces_the_catalog_price_in_the_spend_rate` (including the mixed-turn fallback) and the `modes.rs` first-sample test.
- Per-turn consumer: the cited insights_report.rs lines are test fixtures. The session and per-model pricing breakdowns already read the recorded cost (metrics_sink/mod.rs:729). The only other per-turn pricers are the quota chart and limit learning, covered in the previous commit, and the efficiency reference decomposition (metrics_sink/reference.rs). That decomposition is a catalog-price what-if, so I left it alone and said so in the docs.
- docs/session-coverage.md now describes the HUD behaviour and the reference-decomposition limit. `check_coverage_contract` passes (17/17).
- Not run as one command: the full desktop nextest. I ran it earlier on the previous commit and 10 unrelated agent_config/insights_report/remediation tests failed on this Mac (the bean expected 5). Since then I ran only the hud_token_map, modes and check_coverage_contract tests plus clippy.

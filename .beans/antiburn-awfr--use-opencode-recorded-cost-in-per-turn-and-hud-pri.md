---
# antiburn-awfr
title: Use OpenCode recorded cost in per-turn and HUD pricing
status: todo
type: feature
priority: low
tags:
    - pricing
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

Session cost and Overview use OpenCode's recorded cost for local providers. Per-turn and HUD pricing (`apps/desktop/src-tauri/src/insights_report.rs:1045,1386`) set `recorded_cost_micros: None`, so a local Ollama model shows a catalog price there.

- Read `turn.recorded_cost_micros` into those turn costs.
- Update the "Per-turn cost views ... do not read that column yet" text in `docs/session-coverage.md`.
- AC: a local OpenCode turn shows its recorded cost (normally 0) in per-turn and HUD views.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

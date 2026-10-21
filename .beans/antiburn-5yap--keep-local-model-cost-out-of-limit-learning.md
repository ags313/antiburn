---
# antiburn-5yap
title: Keep local-model cost out of limit learning
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

`apps/desktop/src-tauri/src/store/provider_limit.rs:1814,2083` set `recorded_cost_micros: None`, so limit learning prices local-provider OpenCode turns at catalog prices. Local models have no subscription quota.

- Decide: read the recorded cost, or leave local-provider turns out of limit learning. Implement that choice.
- Update `docs/session-coverage.md`.
- AC: local-model turns add no catalog cost to limit estimates.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

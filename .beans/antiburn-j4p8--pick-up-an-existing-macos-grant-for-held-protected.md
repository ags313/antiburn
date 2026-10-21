---
# antiburn-j4p8
title: Pick up an existing macOS grant for held protected-folder sessions
status: todo
type: feature
priority: normal
tags:
    - scan
    - macos
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

Since the protected-folder hold (`apps/desktop/src-tauri/src/scan/mod.rs:2633`), sessions under ~/Documents, ~/Desktop, or ~/Downloads stop updating until the folder is granted in the app. This happens even when macOS already granted access, because `discover_external_grants` runs only when `probe_protected` is true.

- Find a way to detect an existing OS grant without opening the consent dialog; check how `discover_external_grants` does it first. Record the grant.
- Show the held folders in Repositories, so the user knows why those sessions do not update.
- Update the protected-folder text in `docs/session-coverage.md`.
- AC: with an existing OS grant, held sessions update on the next pass and no dialog opens.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

---
# antiburn-2xhg
title: Show when a long history window falls back to 7 days
status: todo
type: bug
priority: normal
tags:
    - remote
    - desktop
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`list_sessions` (`apps/desktop/src-tauri/src/remote_sessions.rs:813-845`) falls back to the 7-day window on any error, timeouts and SSH failures included. It then records the scan as successful and only logs `remote_history_window_unavailable`, so the user does not learn that their window was not applied.

- Fall back only when the helper rejects the window (an older helper); report other errors as before.
- Record the fallback on the host so Settings says that this helper lists 7 days and needs an update for a longer window.
- Update `docs/remote-sessions.md` and the session-coverage remote section.
- AC: a timeout during a long-window listing shows an error, and an older helper shows the warning and lists 7 days.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, `format`, and `node scripts/check-design-drift.mjs` are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

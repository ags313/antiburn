---
# antiburn-lmfr
title: Add the turn cost column without a numbered migration
status: todo
type: task
priority: high
tags:
    - store
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

Migration v81 (`apps/desktop/src-tauri/src/store/schema.rs:95`, `TURN_SCHEMA_V12_SQL`) exists only in this fork, but it sits in the shared numbered list. The next upstream migration will collide with it again, and the repair in `store/mod.rs:833-848` handles one exact state only. Do this before the next upstream merge that adds a migration.

- Move the column to an idempotent step after the numbered migrations (check `pragma_table_info('turn')`), like upstream's `FINGERPRINT_WORK` step.
- For a database at fork v81 (column present, `user_version` 81, upstream list at 80), set `user_version` back to the upstream count so upstream's next migration runs. Fold the v76 repair into the same check.
- Test a fresh database, an upstream v80 database, a fork v76 database, and a fork v81 database: each ends with the column and the upstream version.
- AC: an upstream merge that adds migrations needs no new repair code.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

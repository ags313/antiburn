---
# antiburn-lmfr
title: Add the turn cost column without a numbered migration
status: completed
type: task
priority: high
tags:
    - store
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T18:42:52Z
parent: antiburn-gt1g
---

Migration v81 (`apps/desktop/src-tauri/src/store/schema.rs:95`, `TURN_SCHEMA_V12_SQL`) exists only in this fork, but it sits in the shared numbered list. The next upstream migration will collide with it again, and the repair in `store/mod.rs:833-848` handles one exact state only. Do this before the next upstream merge that adds a migration.

- Move the column to an idempotent step after the numbered migrations (check `pragma_table_info('turn')`), like upstream's `FINGERPRINT_WORK` step.
- For a database at fork v81 (column present, `user_version` 81, upstream list at 80), set `user_version` back to the upstream count so upstream's next migration runs. Fold the v76 repair into the same check.
- Test a fresh database, an upstream v80 database, a fork v76 database, and a fork v81 database: each ends with the column and the upstream version.
- AC: an upstream merge that adds migrations needs no new repair code.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Completed antiburn-lmfr. Final HEAD: d4580842; implementation: 847a774a. Both commits have DCO sign-offs and no co-author trailers. Rebase onto feat/all-usage-features was already current. Working tree is clean. Never pushed.

Removed the fork-only v81 from the numbered migration list. The turn recorded-cost column is now an idempotent post-migration step checked through pragma_table_info('turn'). A single reconciliation check restores fork v76 to upstream v75 and fork v81 to upstream v80. A persistent marker prevents future upstream versions from being rewound on reopening.

Kept existing migration tests and added coverage for fresh, upstream v80, fork v76, and fork v81 databases, repeated opens, synthetic upstream v81/v82 migrations, and a legacy fork v81 first opened after upstream adds migration 81. All migration tests pass. Future upstream migrations require no additional repair code.

After the operator updated the baseline test exclusion, bob check passed on 847a774a. Committed the final acceptance checkbox in d4580842, then ran bob check again: build PASSED and test PASSED on that final commit, including formatting and clippy with -D warnings. All acceptance checkboxes are checked and committed. Left the confirmed baseline insights_report test unchanged.

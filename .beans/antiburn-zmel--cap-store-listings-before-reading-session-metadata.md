---
# antiburn-zmel
title: Cap store listings before reading session metadata
status: completed
type: bug
priority: normal
tags:
    - remote
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`stores.rs:95` calls `entry_for` (a `session_log_read`) for every log in the window before the 200/1,000 cap applies. Cursor discovery also reads every transcript. With All history, each sync reads every session on the host.

- Sort by modification time and apply the cap first; read metadata only for the kept logs.
- Test with a fake explorer that counts metadata reads.
- AC: a listing reads metadata for at most the cap's number of sessions.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Cursor follow-up: listing and export preparation both use transcript candidates,
without local cross-source deduplication or metadata merges. A synthetic desktop
composer/transcript collision test verifies that each listed Cursor ID exports
with matching title/CWD and the original transcript bytes. The regression test
fails with the old full-discovery preparation path. Metadata-read-count tests remain.

Rebased onto `feat/all-usage-features` and integrated its shared listing deadline.
The deadline also bounds selected metadata reads. Expired or cancelled metadata
reads preserve previously read sessions. Deadline regression tests cover both paths.

## Summary of Changes

Rebased the branch onto feat/all-usage-features through b55a83f0 and resolved both stores.rs conflicts. Final commit: 81b4c4f8, with DCO sign-off. Working tree is clean.

Integrated upstream's shared listing deadline with capped candidate discovery and selected metadata reads. Metadata timeouts preserve already-read sessions; preparation shares its deadline across discovery and metadata. Adapted the upstream discovery fixture to candidate logs and added regressions for expired metadata budgets and cancellation preserving prior sessions. Kept the metadata-read-count and Cursor listing/export collision regressions.

Validation on the final rebased commit: all 81 remote tests pass; remote clippy --all-targets with warnings denied and cargo fmt --all --check pass. bob check passed build and test on 81b4c4f8. All bean acceptance items remain checked. No push performed.

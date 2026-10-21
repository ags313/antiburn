---
# antiburn-6qdp
title: Use one file-URI helper and one percent-encoder
status: completed
type: task
priority: low
tags:
    - engine
    - remote
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`antigravity::file_uri_path` (`crates/antiburn-local/src/discovery/agents/antigravity.rs:757`) is a fourth file-URI helper, beside `uri_to_directory` (`:1142`), `scanner::strip_file_uri_prefix`, and `cursor::normalize_cursor_workspace_path`. `crates/antiburn-remote/src/database.rs:194-238` repeats one percent-encoding loop.

- Add one `paths::file_uri` module in `antiburn-local`, and use one encoder in `database.rs`.
- AC: all callers use the shared helpers, and the tests pass.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Committed 8ce121f8 with DCO sign-off after rebasing onto feat/all-usage-features (already current).

Added paths::file_uri in antiburn-local and routed Antigravity, scanner, and Cursor URI handling through its shared helpers. Kept prefix-only and decoded-path policies explicit. Consolidated the Windows and Unix SQLite URI percent-encoding loops into one encoder, preserving platform-specific escaping.

Added regression coverage for UTF-8, percent escapes, invalid UTF-8, Windows drive paths, localhost URIs, absolute-path validation, and query/fragment escaping. Updated all four acceptance checkboxes and marked the bean completed.

Validation: local nextest 2801 passed, 6 skipped; remote nextest 75 passed; both crates pass Clippy --all-targets with -D warnings and format checks. Tests used a temporary home; the clean local rerun removed inherited Git overrides after an initial unrelated temporary-repository assertion failed. No tests were excluded beyond the existing skips.

bob check passed build and test on the final commit 8ce121f8. No push performed.

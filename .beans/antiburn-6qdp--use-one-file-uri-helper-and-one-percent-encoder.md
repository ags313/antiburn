---
# antiburn-6qdp
title: Use one file-URI helper and one percent-encoder
status: todo
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

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

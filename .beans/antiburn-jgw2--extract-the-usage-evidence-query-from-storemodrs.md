---
# antiburn-jgw2
title: Extract the usage-evidence query from store/mod.rs
status: todo
type: task
priority: low
tags:
    - store
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`apps/desktop/src-tauri/src/store/mod.rs` grew from 3,658 to 3,804 lines. `USAGE_EVIDENCE_SQL` decodes an 11-element positional tuple from `json_group_array(json_array(...))` (`:443-556`).

- Move the query and its decode to `store/usage_evidence.rs`, and use `json_object` with a `#[derive(Deserialize)]` struct.
- AC: the existing usage tests pass, and `store/mod.rs` is smaller.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

---
# antiburn-jgw2
title: Extract the usage-evidence query from store/mod.rs
status: completed
type: task
priority: low
tags:
    - store
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-22T01:29:12Z
parent: antiburn-gt1g
---

`apps/desktop/src-tauri/src/store/mod.rs` grew from 3,658 to 3,804 lines. `USAGE_EVIDENCE_SQL` decodes an 11-element positional tuple from `json_group_array(json_array(...))` (`:443-556`).

- Move the query and its decode to `store/usage_evidence.rs`, and use `json_object` with a `#[derive(Deserialize)]` struct.
- AC: the existing usage tests pass, and `store/mod.rs` is smaller.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Validation: all seven usage-evidence tests pass. The local full run reproduced
the five known failures and three filesystem-watcher timeouts; Bob's build and
test checks pass with its configured baseline exclusions. Clippy with
`-D warnings` and Rust formatting pass. `store/mod.rs` shrank from 3,900 to
3,710 lines.

## Summary of Changes

Rebased onto feat/all-usage-features (already current). Moved the usage-evidence SQL, decoder, and Store method into store/usage_evidence.rs. Replaced the 11-element JSON tuple with json_object fields and a derived Deserialize struct. Updated the existing query-plan test to reference the extracted query. store/mod.rs shrank from 3,900 to 3,710 lines.

All seven usage-evidence tests pass. Clippy with -D warnings and cargo fmt --check pass. The local full suite reproduced the five known failures plus three filesystem-watcher timeouts; Bob's build and test checks passed with its configured baseline exclusions, including the watcher tests. The final bob check passed on 509343a5.

All acceptance boxes are checked and committed. Both commits have DCO sign-offs, no prohibited commit-message content, and no Co-authored-by trailers. Working tree is clean. Nothing was pushed.

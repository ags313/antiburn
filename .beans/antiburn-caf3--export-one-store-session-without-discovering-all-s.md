---
# antiburn-caf3
title: Export one store session without discovering all sessions
status: completed
type: bug
priority: normal
tags:
    - remote
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T23:31:57Z
parent: antiburn-gt1g
---

`stores::prepare` (`crates/antiburn-remote/src/stores.rs:233-242`) runs full discovery and metadata reads for the agent, then filters by session id. Each export is O(n), so an All-history backfill is O(n²) and can hold several GB in memory on the remote host.

- Find the target from its id: OpenCode and Antigravity rows by id, Cursor and Junie by the same path shape listing uses. Admit the found files through the same path checks as today.
- Cursor export locate is transcript-only (`locate_agent_transcript_source`). It must not go through `CursorExplorer::direct_session_source`, so local desktop locate keeps discovery priority (DesktopComposer over agent transcript) and metadata merge. Export therefore does not carry title-based fork observations or the store.db `model` that listing-time discovery merges in; listing and export metadata can differ for those sessions.
- Do not echo listing paths over the remote protocol (`listed` / `sourcePath`): helper and desktop install separately and use `deny_unknown_fields`. Cursor cwd that only exists in `workspace.json` is resolved on locate for that one project key (decode first, then stop at the first matching `workspace.json`).
- Keep the manifest and path validation unchanged.
- Test with a fake explorer that counts reads: one export reads only that session's files (locate-by-id, no discovery).
- AC: exporting one session reads only that session's files.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Rebased onto feat/all-usage-features; kept Cursor transcript candidates for listing and locate-by-id for export; bob check passed. Not pushed.

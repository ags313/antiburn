---
# antiburn-79ny
title: Describe remote store agents in one typed table
status: completed
type: task
priority: normal
tags:
    - remote
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T21:25:45Z
parent: antiburn-gt1g
---

Per-agent string dispatch for remote store agents appears at about 12 sites: `crates/antiburn-remote/src/lib.rs:26-30` (`BASE_AGENTS`, `OPTIONAL_AGENTS`, and a hand-copied `SUPPORTED_AGENTS`), `stores.rs:61,105,151,219,243`, `export.rs:77,161,188-200,257`, and `collector.rs:544`.

- Add one `RemoteStore` table keyed by `AgentKind` in `antiburn-remote`: primary path for a session id, companions, `Db | File` source, and a base flag. Derive the agent lists from it.
- Do the helper side only; the desktop cache is a separate bean.
- AC: adding an agent changes only the table and that agent's reader, and the existing behaviour tests pass.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Commit 4055122a on bob/antiburn-79ny-r357, rebased onto feat/all-usage-features. bob check passes.

- New crates/antiburn-remote/src/remote_store.rs: REMOTE_STORES, a table of RemoteStore { agent: AgentKind, base, format: StoreFormat::{Db, File}, primary: fn(&str) -> String, companions }. Helpers: RemoteStore::for_slug and discovered_kind.
- BASE_AGENTS, OPTIONAL_AGENTS and SUPPORTED_AGENTS are still pub &[&str] consts, now built from the table at compile time. The desktop's use of SUPPORTED_AGENTS and primary_file_name does not change.
- Replaced string dispatch with table lookups: stores::kind_for (removed), export validate / validate_store_bundle companions / primary_file_name / file_name / write_bundle, and the collector list.
- Changed AgentKind::slug in antiburn-local to a const fn so the lists stay constants. The body is unchanged.
- Added 5 unit tests for the table: slug lookup, lists that follow the table, discovered_kind only for non-base agents, no companions on base stores, and Db format matching a .db primary path. All 67 crate tests pass, and clippy -D warnings and fmt --check are clean.
- Per-agent readers stay in stores.rs (store_source, prepare, session-id extraction). Adding an agent means one table row plus that agent's reader.
- StoreFormat is not read on the helper side yet. It is there for the desktop-cache bean (cached_source in remote_cache.rs).
- Left alone: collector.rs analyze_entry picks the source format for codex vs claude. That is base-agent reader logic and was not on the bean's site list.

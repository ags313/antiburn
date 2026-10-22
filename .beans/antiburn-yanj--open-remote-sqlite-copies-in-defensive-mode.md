---
# antiburn-yanj
title: Open remote SQLite copies in defensive mode
status: completed
type: task
priority: low
tags:
    - remote
    - security
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

The desktop opens OpenCode extracts and Antigravity snapshots that a remote host supplied, with only `SQLITE_OPEN_READ_ONLY`. `PRAGMA table_info` accepts views, so a hostile copy can name a recursive view `message` and hang the analysis worker.

- Set `trusted_schema=OFF` and `SQLITE_DBCONFIG_DEFENSIVE`, require the needed names to be tables in `sqlite_master`, and add a progress handler or time limit.
- Apply this to remote cache copies; apply it to all provider database reads if that costs nothing extra.
- Test: a copy with a view named `message` is rejected quickly.
- AC: a hostile copy cannot hang the analysis worker.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml` (4 `platform::git` GPG tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

Implementation: OpenCode and Antigravity analysis and discovery use defensive,
untrusted-schema read-only connections for local databases and remote copies.
Analysis checks table identity before column inspection. Connections have a
100-million instruction budget and a 100-ms busy timeout.
Other providers remain outside this scoped change.

Validation: recursive message/usage views are rejected in under one second;
an unbounded recursive query returns SQLite interruption. Engine nextest passes
all 2,813 tests with fixture-commit signing disabled through process-local Git
configuration. The default Mac environment has six GPG fixture failures.
Engine clippy, both format checks, and coverage/source contract tests pass.
Desktop clippy passes. The full desktop suite passes through `bob check` with
its documented baseline exclusions and isolated test environment. Three native
watcher tests time out in the agent sandbox but pass through bob's check runner.
Both `bob check` build and test gates pass.

## Summary of Changes

Rebased onto feat/all-usage-features (already current). Hardened OpenCode and Antigravity SQLite analysis, discovery, and desktop fingerprint reads for local databases and remote cache copies: trusted_schema=OFF, SQLITE_DBCONFIG_DEFENSIVE, table identity checks, a fixed 100-million-instruction progress budget, and a 100-ms busy timeout.

Added passing recursive-view rejection tests and an unbounded-query interruption test. Valid large-session processing still passes. Updated session/check coverage and checked every acceptance item in the completed bean.

Engine nextest: all 2,813 tests pass with process-local fixture signing disabled. Desktop validation passes through bob's isolated runner with its documented baseline exclusions. Formatting, clippy with -D warnings, and coverage/source contracts pass. Final bob check passed both build and test gates on commit 2134b79a.

Commits 8f231988 and 2134b79a include DCO sign-offs. Working tree is clean. Nothing was pushed.

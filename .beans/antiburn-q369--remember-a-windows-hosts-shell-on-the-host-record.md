---
# antiburn-q369
title: Remember a Windows host's shell on the host record
status: todo
type: task
priority: low
tags:
    - remote
    - windows
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
blocked_by:
    - antiburn-u622
---

`transport.rs:18-39` keeps cmd.exe hosts in a process-global set. A host gets into the set only when a request fails with the English cmd.exe text "is not recognized as an internal or external command". The export path (`ssh_command`, `transport.rs:402`) depends on an earlier request having filled the set. After an app restart the first request goes to the POSIX command, and a non-English Windows is never detected.

- Store the detected shell (or the platform from `Hello`) on `HostRecord`, and choose the command from it.
- Keep the text match only for the first contact with a host, and state the English-only limit in `docs/remote-sessions.md`.
- AC: after an app restart, a request to a known Windows host uses the Windows command with no failed first request.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] `docs/session-coverage.md` / `docs/check-coverage.md` / `docs/remote-sessions.md` match the new behaviour, and `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` passes
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

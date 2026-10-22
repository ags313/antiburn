---
# antiburn-n93i
title: A slow session export fails only that session
status: todo
type: bug
priority: high
tags:
    - remote
    - sync
    - size-s
created_at: 2026-10-22T08:47:10Z
updated_at: 2026-10-22T08:47:10Z
parent: antiburn-nlmp
---

Since 2026-10-21 11:36:50 every mini395 sync has failed with the generic "Some remote sessions could not be synced", and `lastSuccessfulSyncEpoch` has not moved. Read-only helper runs on 2026-10-22 listed 143 sessions fine, but exports ran at about 0.4 MB/s over the current link: 60 MB Claude Code and Codex sessions took 128-166 s, and an 87 MB Antigravity session took 212 s. `export_with_command` (`crates/antiburn-remote/src/transport.rs`, deadline at about line 420, message "Transcript transfer timed out after 180 seconds" at about line 477) gives each export a fixed 180 s. That timeout is not `transport::SessionRejected`, so `sync_snapshot` (`apps/desktop/src-tauri/src/remote_sessions.rs:871`, match at :887) returns it, the whole scan aborts, and `safe_scan_error` (:796) shows the generic message. Every scan stops at the first large session, so nothing after it syncs.

- Give per-session transfer failures their own typed error in `crates/antiburn-remote/src/transport.rs` (for example `SessionTransferFailed`), separate from host-level failures. An export timeout and a broken stream for one export are per-session. SSH that cannot connect (exit 255, `ConnectTimeout`), a cancel, and a full cache stay host-level and still abort the scan.
- In `sync_snapshot` count a per-session transfer failure like `SessionRejected` (`summary.failed`) and continue with the next session. The host then reports "N of M sessions could not be synced", and the scan still records its successful sync.
- Stop the scan after several consecutive per-session failures (for example 3), so an unreachable host does not wait out one deadline per session.
- Test with a fake transport: one export exceeds its deadline, the other sessions import, `summary.failed` is 1, and the host error names 1 failed session; three consecutive failures stop the scan.
- AC: a slow export over its deadline fails only its own session, and the rest of the scan syncs.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `docs/remote-sessions.md` states the transfer limits and failure behaviour
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

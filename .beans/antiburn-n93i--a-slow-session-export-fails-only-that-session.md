---
# antiburn-n93i
title: A slow session export fails only that session
status: completed
type: bug
priority: high
tags:
    - remote
    - sync
    - size-s
created_at: 2026-10-22T08:47:10Z
updated_at: 2026-10-22T10:53:04Z
parent: antiburn-nlmp
---

Since 2026-10-21 11:36:50 every mini395 sync has failed with the generic "Some remote sessions could not be synced", and `lastSuccessfulSyncEpoch` has not moved. Read-only helper runs on 2026-10-22 listed 143 sessions fine, but exports ran at about 0.4 MB/s over the current link: 60 MB Claude Code and Codex sessions took 128-166 s, and an 87 MB Antigravity session took 212 s. `export_with_command` (`crates/antiburn-remote/src/transport.rs`, deadline at about line 420, message "Transcript transfer timed out after 180 seconds" at about line 477) gives each export a fixed 180 s. That timeout is not `transport::SessionRejected`, so `sync_snapshot` (`apps/desktop/src-tauri/src/remote_sessions.rs:871`, match at :887) returns it, the whole scan aborts, and `safe_scan_error` (:796) shows the generic message. Every scan stops at the first large session, so nothing after it syncs.

- Give per-session transfer failures their own typed error in `crates/antiburn-remote/src/transport.rs` (for example `SessionTransferFailed`), separate from host-level failures. An export timeout and a broken stream for one export are per-session. SSH that cannot connect (exit 255, `ConnectTimeout`), a cancel, and a full cache stay host-level and still abort the scan.
- In `sync_snapshot` count a per-session transfer failure like `SessionRejected` (`summary.failed`) and continue with the next session. The host then reports "N of M sessions could not be synced", and the scan still records its successful sync.
- Stop the scan after several consecutive per-session failures (for example 3), so an unreachable host does not wait out one deadline per session.
- Test with a fake transport: one export exceeds its deadline, the other sessions import, `summary.failed` is 1, and the host error names 1 failed session; three consecutive failures stop the scan.
- AC: a slow export over its deadline fails only its own session, and the rest of the scan syncs.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `docs/remote-sessions.md` states the transfer limits and failure behaviour
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Fixed both review findings. `bob check` exits 0 on the new commit.

1. Local storage failures stay host-level. `transport.rs` now copies with `copy_stream`, which maps only read errors to `SessionTransferFailed::StreamBroken`. A write error (ENOSPC and similar) stays a plain error, so the scan aborts. New test `only_a_read_error_is_a_per_session_transfer_failure` uses a failing reader and a failing writer.
2. A completed scan with isolated failures now records the successful sync time. `scan_completed` is true when the scan returned Ok and either nothing failed or at least one session was imported or unchanged. `update_scan_record` takes that flag and keeps the "N of M sessions could not be synced" warning in `last_error`. A scan where every attempted session failed still does not advance the time. New test `a_scan_with_one_slow_export_records_success_and_the_warning` checks the host record (epoch advanced, count and warning kept). I updated the existing partial-scan test, which asserted the old behaviour (the epoch was kept at 10 and is now 20).
3. `docs/remote-sessions.md` now states both behaviours.

The success-time change applies to rejected sessions too, not only transfer failures. The bean's "scan still records its successful sync" read most simply that way. The analytics outcome still reports Failed for partial scans, and I did not change that.

Checks: `antiburn-remote` nextest passes 93/93. The `remote_sessions` tests in the desktop crate pass 29/29. Clippy `-D warnings` is clean on both crates. I did not re-run the full desktop suite directly this round.

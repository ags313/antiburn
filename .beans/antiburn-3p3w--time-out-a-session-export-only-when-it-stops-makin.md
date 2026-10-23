---
# antiburn-3p3w
title: Time out a session export only when it stops making progress
status: completed
type: bug
priority: high
tags:
    - remote
    - sync
    - size-s
created_at: 2026-10-22T08:47:10Z
updated_at: 2026-10-23T01:18:42Z
parent: antiburn-nlmp
blocked_by:
    - antiburn-n93i
---

Since 2026-10-21 11:36:50 every mini395 sync has failed with the generic "Some remote sessions could not be synced", and `lastSuccessfulSyncEpoch` has not moved. Read-only helper runs on 2026-10-22 listed 143 sessions fine, but exports ran at about 0.4 MB/s over the current link: 60 MB Claude Code and Codex sessions took 128-166 s, and an 87 MB Antigravity session took 212 s. `export_with_command` (`crates/antiburn-remote/src/transport.rs`, deadline at about line 420, message "Transcript transfer timed out after 180 seconds" at about line 477) gives each export a fixed 180 s. That timeout is not `transport::SessionRejected`, so `sync_snapshot` (`apps/desktop/src-tauri/src/remote_sessions.rs:871`, match at :887) returns it, the whole scan aborts, and `safe_scan_error` (:796) shows the generic message. Every scan stops at the first large session, so nothing after it syncs.

- Replace the fixed 180 s deadline in `export_with_command` with a progress deadline: fail when no bytes arrive for 60 s, and keep a large total cap (for example 30 minutes) against a stream that trickles forever.
- Make both durations parameters, so tests run in milliseconds.
- Update the timeout message to say which limit fired.
- Test with a fake command: a steady slow stream that takes longer than the old total deadline succeeds; a stream that stops sending fails after the no-progress window; a stream over the total cap fails.
- AC: an export fails only after the no-progress window passes with no bytes, or at the total cap.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `docs/remote-sessions.md` states the transfer limits and failure behaviour
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Rebased onto `feat/all-usage-features` and resolved the conflicts in `crates/antiburn-remote/src/transport.rs`. Upstream added the export-failure tagging (`ExportDestinationFailed`, `ExportTransportFailed`) and its tests; I kept both sides.
- The upstream failure-reason mapping in `remote_sessions.rs` matched the removed `TimedOut` variant. It now maps both `NoProgress` and `TotalCapExceeded` to `SessionFailureReason::Timeout`. This is a new commit on top of the rebased ones.
- The `antiburn-remote` nextest run passes (100 of 100). Clippy with `-D warnings` and `cargo fmt --check` are clean for both crates.
- Desktop nextest has only the 5 known managed-settings failures. `bob check` passes. Nothing pushed.

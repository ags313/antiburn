---
# antiburn-uoqm
title: Log why a remote session failed to sync
status: completed
type: task
priority: high
tags:
    - remote
    - sync
    - logging
    - size-xs
created_at: 2026-10-22T08:47:10Z
updated_at: 2026-10-23T00:32:47Z
parent: antiburn-nlmp
blocked_by:
    - antiburn-n93i
---

Since 2026-10-21 11:36:50 every mini395 sync has failed with the generic "Some remote sessions could not be synced", and `lastSuccessfulSyncEpoch` has not moved. Read-only helper runs on 2026-10-22 listed 143 sessions fine, but exports ran at about 0.4 MB/s over the current link: 60 MB Claude Code and Codex sessions took 128-166 s, and an 87 MB Antigravity session took 212 s. `export_with_command` (`crates/antiburn-remote/src/transport.rs`, deadline at about line 420, message "Transcript transfer timed out after 180 seconds" at about line 477) gives each export a fixed 180 s. That timeout is not `transport::SessionRejected`, so `sync_snapshot` (`apps/desktop/src-tauri/src/remote_sessions.rs:871`, match at :887) returns it, the whole scan aborts, and `safe_scan_error` (:796) shows the generic message. Every scan stops at the first large session, so nothing after it syncs.

- When a session fails to sync, write one warn-level `tracing` event with the host id, agent, session id, and a reason category: rejected or changed, timeout (no progress or total cap), transport, cache.
- Do not log paths, titles, transcript content, or SSH output.
- Test the reason classification for each error kind, and that the event names the session key.
- AC: a failed session produces one warn log that names its session key and reason.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `docs/remote-sessions.md` states the transfer limits and failure behaviour
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Fixed session failure classification (commit 81a671f4); `bob check` exits 0 on it.
- `crates/antiburn-remote/src/transport.rs`: `export_to` now tags its errors by origin. SSH spawn, stdin, stderr, wait, exit and oversize-bundle failures carry `ExportTransportFailed`. Local destination create, write and sync failures carry `ExportDestinationFailed`. `SessionRejected` and `SessionTransferFailed` keep their own types.
- `session_failure_reason` in `remote_sessions.rs` now maps `ExportTransportFailed` to `transport`. `ExportDestinationFailed` and untyped import errors stay `cache`.
- Tests with production error shapes:
  - `export_failures_name_their_origin` drives real `export_with_command` with a fake ssh. It covers exit 255, an oversize bundle, a missing ssh binary (spawn failure), a rejected exit code, and an unwritable destination.
  - `a_destination_write_error_is_tagged_as_local` covers a failing destination writer.
  - The desktop classification test gained transport and destination cases next to the existing rejection, timeout, stream and import cases.
- Remote crate: 95 tests, clippy, fmt clean. Desktop: the `remote` filter (74 tests), clippy, fmt clean.
- `docs/remote-sessions.md` now defines transport vs cache.

---
# antiburn-nlmp
title: Keep remote sync working over a slow link
status: todo
type: epic
priority: high
tags:
    - remote
    - sync
created_at: 2026-10-22T08:47:10Z
updated_at: 2026-10-22T08:47:10Z
---

Since 2026-10-21 11:36:50 every mini395 sync has failed with the generic "Some remote sessions could not be synced", and `lastSuccessfulSyncEpoch` has not moved. Read-only helper runs on 2026-10-22 listed 143 sessions fine, but exports ran at about 0.4 MB/s over the current link: 60 MB Claude Code and Codex sessions took 128-166 s, and an 87 MB Antigravity session took 212 s. `export_with_command` (`crates/antiburn-remote/src/transport.rs`, deadline at about line 420, message "Transcript transfer timed out after 180 seconds" at about line 477) gives each export a fixed 180 s. That timeout is not `transport::SessionRejected`, so `sync_snapshot` (`apps/desktop/src-tauri/src/remote_sessions.rs:871`, match at :887) returns it, the whole scan aborts, and `safe_scan_error` (:796) shows the generic message. Every scan stops at the first large session, so nothing after it syncs.

The three child beans make one slow session fail alone, time an export out on stalled progress instead of total time, and log which session failed and why. They touch the same files, so they run in order.

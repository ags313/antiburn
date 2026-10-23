---
# antiburn-6z67
title: Log why a remote scan failed
status: completed
type: bug
priority: high
tags:
    - remote
    - sync
    - logging
    - size-xs
created_at: 2026-10-23T09:00:19Z
updated_at: 2026-10-23T16:23:26Z
parent: antiburn-nlmp
---

Both remote hosts show "Last sync failed · Some remote sessions could not be synced" (2026-10-23 ~09:50; mini395 last success 05:19:43 on build c3e5b246, tr9985 last success 2026-10-16). That message comes from `safe_scan_error` (`apps/desktop/src-tauri/src/remote_sessions.rs:796`), which hides every scan-level error. Nothing in the app log explains it: no `remote_session_sync_failed` event and no line from the remote modules since the 04:40 restart. Read-only replays with the installed helpers work end to end: hello, list (mini395 143 sessions in 0.5 s, tr9985 126 in 1.9 s), and every export (mini395 1.4 GB, tr9985 315 MB, all exit 0, slowest 2.2 s). So the failing step is on the desktop, outside the per-session sync loop that already logs: snapshot validation, cache budget, import, the snapshot or host record write, or the store write.

- In `perform_scan` (`remote_sessions.rs:1028`), write one warn-level event `remote_scan_failed` whenever the scan ends with an error: host id, the stage that failed (hello, list, validate, budget, sync, snapshot write, record write, store, or another stage the code has), the session key when the stage concerns one session, and the full error chain (`{error:#}`).
- This is the operator's local debug log: local cache paths in the chain are acceptable; never log transcript content, titles, or SSH output.
- Keep `safe_scan_error` for the UI unchanged.
- Test: a fake transport or store that fails at two different stages produces the event with the right stage and the error chain.
- AC: the next failing scan on either host writes one `remote_scan_failed` line that names the stage and the error.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

- `perform_scan` tracks its stage (hello, list, cache_directory, sync, finish) and the last session that failed, and writes one warn event `remote_scan_failed` when the scan ends with an error other than a cancel: host id, stage, agent and session id when a session failed, the error chain, and the SSH transport's bounded local diagnostic (`PrerequisiteError::diagnostic`, which its API reserves for local use: not UI or analytics).
- `safe_scan_error` and the host row are unchanged.
- Test: `a_failed_scan_logs_its_stage_session_and_error_chain` (list and sync stages, cancelled scan not logged). Desktop suite: 3,328 passed; the 5 known managed-settings tests fail on this Mac as before.

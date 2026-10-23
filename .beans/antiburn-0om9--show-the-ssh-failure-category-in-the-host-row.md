---
# antiburn-0om9
title: Show the SSH failure category in the host row
status: todo
type: bug
priority: high
tags:
    - remote
    - sync
    - size-s
created_at: 2026-10-23T20:47:37Z
updated_at: 2026-10-23T20:47:37Z
parent: antiburn-nlmp
---

On 2026-10-23 both remote hosts showed only "Last sync failed · Some remote sessions could not be synced" for hours. The real cause was an SSH failure at the hello step (`ssh: connect to host 10.0.10.39 port 22: No route to host`), which only `antiburn-6z67`'s new `remote_scan_failed` log showed. `safe_scan_error` (`apps/desktop/src-tauri/src/remote_sessions.rs:796`) maps every scan-level error except cache-full and cancel to `TransferFailed` with that generic message. `preflight_error` (`:364`) already maps each `transport::PrerequisiteError` category to a status and a plain message ("SSH could not reach this host", "SSH key authentication failed for this host", ...), and `RemoteErrorCategory` (`:54`) already has the matching variants.

- When the scan error chain holds a `PrerequisiteError`, set the host row's `lastError` from its category: the matching `RemoteErrorCategory` and the same message the preflight shows. Use one shared mapping for both paths, not a copy.
- When the category is SSH-unavailable and the diagnostic says "No route to host" for a private (RFC 1918) address, add a short hint: macOS can block an app's local network access (System Settings → Privacy & Security → Local Network). Keep the raw diagnostic out of the UI, as the transport API asks.
- Keep the generic message for other scan errors; keep the per-session "N of M sessions could not be synced" message.
- Check how the frontend shows `lastError` (`RemoteHostsSection.tsx:618`) and any analytics that read the category, and update their tests.
- Test: a scan that fails at hello with each prerequisite category gives the right category and message; the "No route to host" case on a private address adds the hint; a non-prerequisite error keeps the generic message.
- AC: an SSH failure at hello shows its own category message in the host row, and the local-network case names the macOS setting.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, and `format` are clean
- [ ] `docs/remote-sessions.md` describes the host-row error messages
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

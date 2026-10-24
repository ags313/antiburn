---
# antiburn-0om9
title: Show the SSH failure category in the host row
status: completed
type: bug
priority: high
tags:
    - remote
    - sync
    - size-s
created_at: 2026-10-23T20:47:37Z
updated_at: 2026-10-24T10:42:19Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] `pnpm --filter @antiburn/desktop lint`, `type-check`, `test`, and `format` are clean
- [x] `docs/remote-sessions.md` describes the host-row error messages
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

A scan that fails on an SSH prerequisite now shows that category's message in the host row instead of the generic one. I ran the Rust and desktop checks and `bob check` after committing, and all passed. I did not run the full desktop nextest suite; I ran the 35 `remote_sessions` tests, which pass.

- **Shared mapping:** `prerequisite_failure` in `remote_sessions.rs` maps each `PrerequisiteError` category to a `RemoteErrorCategory` and message. Both `preflight_error` and `safe_scan_error` use it. `safe_scan_error` looks through the whole error chain for a `PrerequisiteError`.
- **Local-network hint:** when the category is SSH-unavailable and the diagnostic says "No route to host" for an RFC 1918 IPv4 address, the message adds the macOS Local Network setting hint. The raw diagnostic never reaches the UI. The hint also appears in the Check connection result, since both paths share the mapping.
- **Unchanged messages:** other scan errors keep the generic message, and the "N of M sessions" message is untouched.
- **API change:** I made `PrerequisiteError::new` public so the app tests can build one.
- **Frontend and analytics:** the frontend shows `lastError.message` as is, so I changed no code and added one test for the hint message. No analytics code reads this category.
- **Tests:** Rust tests cover each prerequisite category, the private-address hint, no hint for a public address or for another SSH error, and the generic message for non-prerequisite errors.
- **Docs and bean:** `docs/remote-sessions.md` now describes the host-row messages. The acceptance boxes in the bean are ticked.
- **Rebase:** the branch was already up to date with `feat/all-usage-features`.

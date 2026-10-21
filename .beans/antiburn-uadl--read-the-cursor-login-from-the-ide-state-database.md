---
# antiburn-uadl
title: Read the Cursor login from the IDE state database
status: todo
type: task
priority: normal
tags:
    - cursor
    - usage
    - size-s
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
parent: antiburn-xupl
---

The Cursor IDE keeps its login in `state.vscdb` (`ItemTable`, key `cursorAuth/accessToken`, a JWT with `exp`). The spike read it from `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`.

- Add `provider_usage/live/sources/cursor_login.rs`: open the database read-only, read the access token, and check `exp` with no network call.
- Return a typed state: signed in (with the token), signed out, expired, or unreadable. Support the macOS, Linux (`~/.config/Cursor/...`), and Windows (`%APPDATA%\Cursor\...`) paths; only macOS was seen.
- Test with synthetic `state.vscdb` files for each state.
- Never print, log, or store the Cursor token; send it only to `api2.cursor.sh`; never use `cursorAuth/refreshToken` (a refresh can sign the Cursor IDE out). Test fixtures are synthetic: no real usage data, ids, or tokens.
- AC: the reader returns the token only for a present, unexpired login, and the tests cover each state.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

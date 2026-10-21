---
# antiburn-u622
title: Quote the helper path in the Windows helper command
status: todo
type: bug
priority: normal
tags:
    - remote
    - windows
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`WINDOWS_HELPER_COMMAND` (`crates/antiburn-remote/src/transport.rs:14`) is `%USERPROFILE%\.local\bin\antiburn-remote.exe stdio` with no quotes. OpenSSH runs it through `cmd.exe /c`, which splits `C:\Users\First Last\...` at the space and reports "is not recognized". `classify_failure` then shows HelperMissing for a helper that is installed.

- Quote the path: `"%USERPROFILE%\.local\bin\antiburn-remote.exe" stdio`.
- Test the command string, and that the not-recognized classification still works for the quoted form.
- Check the Windows install text in `docs/remote-sessions.md`.
- AC: a Windows profile path that contains a space runs the helper.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

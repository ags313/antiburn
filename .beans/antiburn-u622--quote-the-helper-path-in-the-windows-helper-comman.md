---
# antiburn-u622
title: Quote the helper path in the Windows helper command
status: completed
type: bug
priority: normal
tags:
    - remote
    - windows
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T21:07:09Z
parent: antiburn-gt1g
---

`WINDOWS_HELPER_COMMAND` (`crates/antiburn-remote/src/transport.rs:14`) is `%USERPROFILE%\.local\bin\antiburn-remote.exe stdio` with no quotes. OpenSSH runs it through `cmd.exe /c`, which splits `C:\Users\First Last\...` at the space and reports "is not recognized". `classify_failure` then shows HelperMissing for a helper that is installed.

- Quote the path: `"%USERPROFILE%\.local\bin\antiburn-remote.exe" stdio`.
- Test the command string, and that the not-recognized classification still works for the quoted form.
- Check the Windows install text in `docs/remote-sessions.md`.
- AC: a Windows profile path that contains a space runs the helper.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Committed 30aaf38d (DCO signed): quote the Windows helper executable path so profile paths containing spaces remain one command path. Rebased onto feat/all-usage-features (already current).

Added regression coverage for the exact quoted SSH command and HelperMissing classification for a quoted path containing First Last. Updated the Windows setup command in docs/remote-sessions.md and checked all acceptance items in the antiburn-u622 bean.

Validation: all 65 antiburn-remote nextest tests passed; cargo clippy --all-targets -- -D warnings and cargo fmt --all --check passed for the crate; doctests passed (0 tests). bob check passed on the final commit. Native Windows execution was not available on this macOS host; regression tests validate command construction and diagnostics. An optional ps diagnostic was denied by the sandbox and was not needed for validation. No push performed.

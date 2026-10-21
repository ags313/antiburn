---
# antiburn-8a0s
title: Windows helper copies each bundle file from its start
status: completed
type: bug
priority: high
tags:
    - remote
    - windows
    - size-xs
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T18:25:52Z
parent: antiburn-gt1g
---

On Windows, `read_prefix` (`crates/antiburn-remote/src/evidence.rs:255-277`) reads with `FileExt::seek_read`, which moves the file cursor to the end of the read (Rust std docs). `collector.rs:211/217` (`read_origin`, `read_preview`) and `export.rs:480` (`read_label`) read the admitted handle before `write_prepared_bundle` copies it with `Read::by_ref(&mut source.file).take(size)` (`export.rs:540-548`). The copy starts mid-file, `copied < size`, and the export fails with "Session changed during transfer; retry". So every Claude Code and Codex export from a Windows host fails. Linux is not affected: `read_at` keeps the cursor.

- Seek each `StorePayload::File` to `SeekFrom::Start(0)` before the copy on every platform, or copy with positional reads.
- Correct the comment above the Windows `read_prefix`: the read does move the shared cursor.
- Test on every platform: read a preview from the handle, move the cursor, then write the bundle; the bundle holds every byte.
- AC: a bundle written after a preview read holds the full file on Linux, macOS, and Windows.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo nextest run --manifest-path crates/antiburn-remote/Cargo.toml`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` for that crate are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

## Summary of Changes

Committed fc78c380 with DCO sign-off: fix(remote): rewind admitted files before bundle transfer. No prohibited commit message text or Co-authored-by trailer. No push.

Every StorePayload::File now seeks to SeekFrom::Start(0) before copying on all platforms. Corrected the Windows read_prefix comment to state that reads move the shared cursor. Added a platform-independent regression that reads previews from parent and companion handles, moves both cursors, writes the bundle, and verifies all payload bytes and manifest sizes.

All three acceptance checkboxes are checked in the committed antiburn-8a0s bean. Rebase onto feat/all-usage-features was already current. Remote-crate validation passed locally on macOS: 63 nextest tests, clippy --all-targets -- -D warnings, cargo fmt --all --check, and doctests. The regression has no platform gate and existing CI covers Linux, macOS, and Windows.

bob check passed build and test on the last commit fc78c380 after the operator excluded the confirmed baseline desktop test failure. That test was not changed. The working tree was clean; no further changes were needed.

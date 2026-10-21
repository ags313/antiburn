---
# antiburn-yt52
title: Document Cursor account usage in the coverage docs
status: todo
type: task
priority: normal
tags:
    - cursor
    - docs
    - size-xs
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-21T17:15:42Z
parent: antiburn-xupl
blocked_by:
    - antiburn-2ol7
---

`docs/session-coverage.md` says Cursor transcripts carry no usage. The account source changes what the product can show.

- Describe the source: opt-in, Cursor IDE login, private undocumented endpoint, fields read, no cache-write tokens, model `default` for Auto, `conversationId` join, and account-level usage.
- State in `docs/check-coverage.md` that no check uses this source (usage only), if the coverage rules need it.
- Mention the Cursor account in the remote-sessions text: remote Cursor sessions get usage from this Mac's login.
- AC: the docs match the shipped behaviour and the contract test passes.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

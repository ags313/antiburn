---
# antiburn-yt52
title: Document Cursor account usage in the coverage docs
status: completed
type: task
priority: high
tags:
    - cursor
    - docs
    - size-xs
created_at: 2026-10-21T17:15:42Z
updated_at: 2026-10-22T00:53:52Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `cargo test --manifest-path crates/antiburn-local/Cargo.toml --test check_coverage_contract` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: Cursor account usage spike, `.claude/worktrees/preview/.agent-artifacts/plans/2026-10-21-cursor-usage-spike.md` (local, not committed).

## Summary of Changes

Documented Cursor account usage in coverage docs: opt-in IDE-login feed via the private GetFilteredUsageEvents endpoint, reviewed fields (no cache-write; model `default` for Auto), conversationId join and account-level leftovers, and that no check uses this usage-only source. Updated remote-sessions text so remote Cursor sessions get tokens/cost from this Mac's login. Rebased onto feat/all-usage-features (already current). check_coverage_contract and bob check passed on 03254d80.

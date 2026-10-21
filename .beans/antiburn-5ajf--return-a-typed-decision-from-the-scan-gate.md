---
# antiburn-5ajf
title: Return a typed decision from the scan gate
status: todo
type: task
priority: normal
tags:
    - scan
    - refactor
    - size-s
created_at: 2026-10-21T10:08:23Z
updated_at: 2026-10-21T10:08:23Z
parent: antiburn-gt1g
---

`apply_repo_admission` (`apps/desktop/src-tauri/src/scan/mod.rs`, about line 2698) takes 7 parameters, 4 of them `&mut` side channels, and `apply_prepared_admission` (`scan/prepare.rs:255-282`) works out the outcome from whichever collection changed. The no-CWD folder rule is copied into the test-only `describe_with_gate` (`scan/mod.rs:2336-2357`).

- Return `GateDecision { Admit(Option<root>) | Reject | Hold(HoldReason::{Git(error), Consent(cwd), FolderGone}) }`, and let callers fold it into their counters.
- Add `impl AddAssign for GateCounts` to replace the field-by-field merge in `owner.rs:694-700`.
- Pass "full current-window pass" to `record_consent_held` as its own value, not as `policy.progress`.
- AC: the scan tests pass with the same behaviour, and no outcome is worked out from out-parameters.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

Source: thermos review of `feat/all-usage-features` at 3ef43383, `.claude/worktrees/preview/.agent-artifacts/reviews/2026-10-21-thermos/report.md`.

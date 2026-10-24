---
# antiburn-0jci
title: Show the Gemini weekly limit as a weekly window
status: todo
type: bug
priority: high
tags:
    - provider-usage
    - antigravity
    - size-s
created_at: 2026-10-24T09:36:51Z
updated_at: 2026-10-24T09:36:51Z
---

On 2026-10-24 the `agy` CLI reported 100% of the Gemini weekly allowance used, while antiburn had no Gemini weekly window at all. The dev database (`provider_usage_observation`, provider `google`, source `antigravity-usage-fetch`) shows two faults:

- `antigravity-gemini-weekly` was last observed on 2026-10-15 14:35 (15 readings, 32–42% used). Every fetch since then has no Gemini weekly window.
- `antigravity-gemini-5h` (role `primaryShort`) has carried the reset `2026-10-24 05:49:18 BST` since at least 2026-10-21 11:16, which is about three days ahead. Its use climbed from 73.9% to 98.75% by 2026-10-21 22:50, and the fraction then disappeared (exhausted; `0ff0fe45` now reads that as 100%). A 5-hour window cannot have a reset three days ahead, so this reading is the weekly limit shown with the 5-hour label.

The Claude + GPT pools are not affected: their 5-hour and weekly windows are both present, at 0%, with resets 5 hours and 7 days ahead.

Hypothesis (not confirmed): around 2026-10-15 Google changed the `retrieveUserQuotaSummary` Gemini group. It now returns either one bucket whose id matches `5h` and whose reset follows the binding limit, or a weekly bucket whose id and group name do not contain `weekly`, `week`, or `7d`. `semantic_slot` (`apps/desktop/src-tauri/src/provider_usage/live/antigravity.rs`) drops a bucket it cannot classify. The summary then has fewer than 4 windows, and the `fetchAvailableModels` fallback adds no Gemini weekly window because no model quota names a week.

- Capture first: add an ignored live test (or a debug-only command) that uses the existing credential reader and prints, for `retrieveUserQuotaSummary` and `fetchAvailableModels`, only the group names, bucket ids, window ids and labels, fractions, and reset times. It must never print or store the access token, refresh token, or project id. Keep the capture under `.agent-artifacts/`.
- From the capture, fix the classification so the Gemini weekly limit maps to `antigravity-gemini-weekly` and the 5-hour limit to `antigravity-gemini-5h`. Do not guess from the reset distance alone unless the capture shows no other signal; if you do, state that limit in a comment and in the tests.
- If Google now sends only one Gemini bucket, show it with the window kind that its reset supports, and keep the other Gemini window absent (unknown), not 0%.
- Add a synthetic fixture with the captured shape (synthetic values, no account data) and tests for both Gemini windows, including an exhausted weekly bucket with no fraction.
- AC: when `agy` reports the Gemini weekly limit at N%, antiburn shows a Gemini weekly window at N% with the same reset, and the Gemini 5-hour window never has a reset more than 5 hours ahead.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, and `cargo fmt --check` are clean
- [ ] no token, project id, or real account data is in the repository
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

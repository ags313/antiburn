---
# antiburn-0jci
title: Show the Gemini weekly limit as a weekly window
status: completed
type: bug
priority: high
tags:
    - provider-usage
    - antigravity
    - size-s
created_at: 2026-10-24T09:36:51Z
updated_at: 2026-10-24T13:03:58Z
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

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, and `cargo fmt --check` are clean
- [x] no token, project id, or real account data is in the repository
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

Addressed all three review points in commit fd315cbc, on top of e3c19bb9.

1. **Pools before combining.** `merge_windows` now takes `now`. Each unnamed Gemini model quota is classified on its own reset: a reset beyond 5h+60s feeds the weekly pool, otherwise the 5h pool. A pool keeps the percentage and reset of the single most-used quota, never independent maxima. Your example (90% at +2h, 40% at +3d) now gives 5h at 90%/+2h and weekly at 40%/+3d. Tests run through `fetch_cloud` with both endpoints:
   - the captured shape, a summary with only the Claude+GPT group and Gemini `quotaInfo` fallbacks
   - an exhausted weekly quota with no fraction
   - a summary `5h` bucket with a weekly reset
   `WEEKLY_AS_FIVE_HOUR` remains as one parser-level test only.
2. **Final-hours ambiguity.**
   - A weekly-labelled bucket never moves to 5h. A test checks its identity at 3 days, 5h09m, 4h, and 1 minute before the reset.
   - Only a reset beyond the limit rules out the short window. The limit is 5h plus 60s of skew, replacing the 5h10m margin, and the boundary test covers 60s, 4h59m, 5h, 5h+60s, 5h+61s and 3d. Every non-empty case asserts the short window is within the limit.
   - **Limit:** an unlabelled model quota with a near reset still reads as 5h. A weekly limit in its last hours looks the same, and no state across fetches is kept. The rule is documented in a code comment and in the test `a_lone_unlabelled_near_reset_model_quota_stays_a_five_hour_reading`. Say if you want such readings left unknown instead.
3. **Capture.** `capture_report` reads fixed paths only: group `displayName`, bucket `bucketId`/`displayName`/`window`, and `quotaInfo`/`quotaInfos` fields `windowId`/`windowLabel`/`remainingFraction`/`resetTime`. The `capture_report_keeps_only_quota_fields` test uses a synthetic project.id, credential objects and arbitrary keys, and checks none reach the output. I regenerated the artifact in `.agent-artifacts/reviews/` with it.

Checks: the nextest run shows 3349 passed and the same 5 failures as before, none in this work, clippy with `-D warnings` is clean, `cargo fmt --check` is clean, and `bob check` printed a passing summary.

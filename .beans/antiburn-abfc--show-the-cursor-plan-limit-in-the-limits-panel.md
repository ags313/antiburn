---
# antiburn-abfc
title: Show the Cursor plan limit in the limits panel
status: todo
type: feature
priority: normal
tags:
    - cursor
    - provider-usage
    - size-s
created_at: 2026-10-24T21:31:36Z
updated_at: 2026-10-24T21:31:36Z
---

On 2026-10-24 the limits panel (`apps/desktop/src/views/main-window/overview/OverviewProviderLimits.tsx`) showed Claude, Google, and Codex but no Cursor. The panel lists only providers whose live usage source reports limit windows. The Cursor account source (`apps/desktop/src-tauri/src/provider_usage/live/sources/cursor_fetch.rs`, parser `live/cursor.rs`) reads only `GetFilteredUsageEvents` for tokens and cost. It reports no plan allowance and no reset.

Candidate source, found by name only in the installed Cursor app's bundled message definitions (`/Applications/Cursor.app/Contents/Resources/app/out/vs/code/electron-utility/alwaysLocalSingleton/alwaysLocalSingletonMain.js`): `aiserver.v1.DashboardService/GetCurrentPeriodUsage`. Its response definition names `billing_cycle_start`, `billing_cycle_end`, `plan_usage`, `spend_limit_usage`, `total_spend`, `included_spend`, `bonus_spend`, `remaining`, `limit`, `auto_spend`, `api_spend`, `auto_limit`, `api_limit`, `auto_percent_used`, `api_percent_used`, and `total_percent_used`. The same bundle also names `GetPlanInfo` (`plan_name`, `included_amount_cents`, `billing_cycle_end`) and `GetCurrentBillingCycle` (`start_date_epoch_millis`, `end_date_epoch_millis`). Hypothesis: these fields nest as `planUsage { ... }` in Connect JSON and show the included plan usage for the current billing cycle. Nobody has called them yet.

Spike (about 30–60 minutes):
- Call `GetCurrentPeriodUsage` (and `GetPlanInfo` if needed) with the same login and HTTP client as `GetFilteredUsageEvents` (`cursor_login.rs`, `sources/http.rs`). Never print, log, or store the access token. Never use the refresh token.
- Record only the field names, nesting, value types, and numbers under `.agent-artifacts/`, with no account or team ids. Compare `total_percent_used` (or `included_spend` / `limit`) and the billing-cycle end with the Cursor dashboard's usage page. Write the result in this bean and label it: it confirms or refutes the hypothesis above.
- If neither call gives a provider-stated allowance and reset, stop and report. Do not compute a limit from the plan price.

Feature (if the spike confirms a source):
- Add a Cursor live usage source behind the existing Cursor account opt-in. It reports one `PrimaryLong` window for the included plan usage, with the provider-stated percent and the billing-cycle end as the reset. Connect JSON may omit zero values; read a missing number as 0 only where the spike shows that.
- Add separate `Supplemental` windows for the Auto and API pools only if the spike shows that Cursor reports them with their own limits.
- Use a billing-cycle window kind. Check how the panel and the tray popover label `UsageWindowKind::Other`; add a named kind only if the label needs it.
- Show the plan name from the reply, as the other providers do.
- Tests: a strict parser with synthetic fixtures (missing zero values, malformed present values reject the reply), the source's opt-in gate, and the panel rendering a Cursor group.
- AC: with the Cursor account opt-in on, the limits panel shows a Cursor group whose percent and reset match the Cursor dashboard.

## Acceptance

- [ ] the spike result is written in this bean and labelled confirmed or refuted
- [ ] if confirmed: the AC above holds and the tests that cover it exist and pass
- [ ] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check`, and `pnpm --filter @antiburn/desktop lint`, `type-check`, and `test` are clean
- [ ] no token, account id, or real account data is in the repository
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

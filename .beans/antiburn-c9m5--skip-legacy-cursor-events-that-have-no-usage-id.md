---
# antiburn-c9m5
title: Skip legacy Cursor events that have no usage id
status: completed
type: bug
priority: high
tags:
    - cursor
    - usage
    - size-xs
created_at: 2026-10-22T08:31:46Z
updated_at: 2026-10-22T08:37:45Z
parent: antiburn-xupl
---

With the Cursor account usage opt-in on, the app stored no events and Overview showed no Cursor usage. The app log had one `cursor_usage_fetch_failed` with `category="schema"`. The first fetch starts at the retention cutoff (0 with unlimited retention), so it reaches the account's oldest events. On this account, page 12 holds 12 events from 2024-10 to 2025-03 with kind `USAGE_EVENT_KIND_CUSTOM_SUBSCRIPTION` that have no `usageEventId` and no `tokenUsage` (0 charged cents). `parse_event` requires `usageEventId`, so the page fails with `MissingRequiredField`, `fetch_pages` discards the run, and every retry fails the same way. The spike checked only 400 days and missed these events.

- Skip an event that has neither `usageEventId` nor `tokenUsage`, and count it in the page. An event with `tokenUsage` but no `usageEventId` still rejects the page.
- Count skipped events toward `totalUsageEventsCount` in `fetch_pages`, so a run that ends with legacy events completes.
- Tests: the parser skips a legacy event and still rejects an unidentified event with tokens; a fetch whose last page holds legacy events completes and stores the other events.
- AC: a first fetch over an account history that ends with legacy events stores every identified event.

## Acceptance

- [x] the AC above holds and the tests that cover it exist and pass
- [x] `env -u CLAUDE_EFFORT cargo nextest run --manifest-path apps/desktop/src-tauri/Cargo.toml` (5 managed-settings tests fail on this Mac regardless), clippy with `-D warnings`, `cargo fmt --check` are clean
- [x] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

## Summary of Changes

- `parse_usage_page` skips an event that has neither `usageEventId` nor `tokenUsage` and reports it in `CursorUsagePage::skipped`; an event with `tokenUsage` but no id still rejects the page.
- `fetch_pages` counts listed events (stored plus skipped) toward `totalUsageEventsCount`, so a run whose last page holds legacy events completes.
- Tests: two parser tests and one fetch test. A local check of the real account history parsed 1,186 events and skipped 12 legacy events, which matched the stated total of 1,198.
- `docs/session-coverage.md` describes the legacy events.

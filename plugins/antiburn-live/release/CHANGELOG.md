# Changelog

## 0.1.2 — unreleased candidate

- Automatically fetches account usage after Desktop attaches when native data is empty or unavailable.
- Uses the host's opaque bearer handle; credentials stay on the host and no request body is sent.
- Keeps valid native readings primary, including partial windows, without account fill.
- Shares one request, reuses successful readings for 60 seconds, and backs off failures with HTTP 429 Retry-After support.
- Preserves prior readings on failure and rejects late results after native events, reset, or detach.
- Shows Checking usage... during a read without numeric usage, then Usage unavailable with a refresh icon if no reading is found.
- Shows one loading status without a duplicate pending control and deduplicates repeated clicks.
- Shows account availability in the runtime timezone with an explicit zone and UTC fallback; combines retry timing and session-refresh guidance in one wrapping line.
- Uses dim Bitcount secondary descriptions on Desktop while retaining native Refresh accessibility and terminal font support.
- Keeps terminal-only usage native; adds no prompt/model calls, usage-fetch or retry timers, polling, or cache/file access.
- Animates pending dots with Desktop CSS honoring reduced motion and a terminal client timer scoped to the mounted indicator.
- Leaves the published catalog pin unchanged; this version requires separate release approval.

## 0.1.1 — previous candidate

- Uses the display name antiburn (Mod), with a separate local dev label.
- Re-reads native usage after a Desktop client attaches.
- Adds an unavailable-state Refresh action and `/antiburn-mod refresh`.
- Shows `[refresh]` in the command hint and retains the `details` argument.
- Makes no prompt or model request to refresh usage; missing host data stays unknown.

## 0.1.0 — initial candidate

- Introduces the `antiburn-mod` identity and `/antiburn-mod` command.
- Shows native five-hour and weekly account usage, reset times, and pace.
- Preserves logo website links, quota hover reveals, and keyboard activation.
- Keeps missing data and stale readings distinct from fresh quota measurements.
- Distributes only the usage implementation, with an included MIT license,
  Bitcount attribution, and the existing antiburn app icon.

This is a release candidate. Native host and lifecycle verification, an
immutable remote source, Directory validation, and publication remain pending.

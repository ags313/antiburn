# Changelog

## 0.1.4 — testing candidate

- Reduces desktop labels to 14px, dots to 7px, and branding to 13px.
- Softens filled dots, delays stacking until 480px, and tightens stacked and status layouts.
- Shows whole-number desktop percentages without rounding into exhaustion.
- Uses steady red dots and pace markers at 100% usage and above.
- Uses solid pace markers with a subtle right-edge shadow drawn once per meter.
- Keeps narrow desktop accessibility labels consistent with visible percentages.
- Preserves exact source readings and terminal formatting.

## 0.1.3 — previous testing candidate

- Keeps meter graphics stable during unrelated redraws by using pixel-aligned pace markers.
- Adds 4 px of space above Desktop usage and one row above terminal usage.
- Omits the extra gap in bands below eight rows or 24 columns to preserve content.
- Targets the testing branch marketplace without a GitHub Release.

## 0.1.2 — previous testing candidate

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
- Distributes through an immutable source in the testing branch catalog.

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

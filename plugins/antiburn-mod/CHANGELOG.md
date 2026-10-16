# Changelog

## 0.1.1 — unreleased

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

# Burn Checks architecture

Burn Checks reuse native source readers, a shared engine, the TypeSafe/Jev
provider, SQLite persistence, and the existing Checks UI. The
[authoring guide](authoring.md#supported-subset) states supported capabilities;
[session coverage](../session-coverage.md#burn-check-source-facts) owns source limits.

## Ownership and dataflow

| Layer             | Owns                                                                                                                                               |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `antiburn-local`  | Discovery, parsing, native identity/order, tool semantics, and source evidence                                                                     |
| `antiburn-checks` | Compilation, context preparation, selection, decisions, actions, and engine transitions                                                            |
| Desktop           | Watcher lifetime, committed evidence publication, SQLite Store adapter, credentials/provider admission and accounting, Tauri API, and presentation |

```text
existing discovery and SessionReader
  → ToolResultObservation and UserMessage
  → committed publication with frozen ToolResultContext
  → StorePublishedEvidenceReader → FileEventSource
  → frozen candidate → DecisionEvaluator
  → atomic assessment/actions/optional status/watermark
  → report event → existing Checks collection and detail pane
```

`CheckEngine` owns transitions. Expression evaluation and action planning remain
pure. Watch notifications request reconciliation; they are not result events.
Use the existing parse pass, not a second transcript reader.

## Identity, order, and frozen evidence

A definition has a filename-stem ID, installation scope, and content revision.
An activation binds its effective definition, environment/session scope, run,
and durable generation. Session branches retain exact source ownership; a Git
branch name is metadata. A collection row groups by logical ID after resolving
project overrides; it does not merge branch status or evidence.

Temporary internal `InstalledCheck*` names and the `installed:` display/snooze
namespace distinguish these rows from native detector IDs. The product term is
**Burn Checks**. These internal discriminators do not extend `DetectorId` or
authorize native remediation.

Native source order selects records. Publication cursors track delivery; engine
positions track committed observations. They are not interchangeable. The first
terminal result can trigger once. A correction keeps its logical result identity
and first-terminal position, but changes its evidence version. Duplicate delivery
is not a new attempt. Timestamps cannot replace proven source order.

The local sink freezes user-message prefixes and main-session admission in
`ToolResultContext`. Candidates freeze selected records, linked calls, scope,
positions, completeness, and revisions. Delayed provider work does not reread
mutable source rows. Missing required evidence prevents assessment. Selected
corrections/deletions can invalidate current status without writing success;
unrelated appends do not invalidate a candidate. Project reads filter exact
environment/session CWD before the page limit. Old payloads without context stay
unavailable.

## Execution and persistence

Discovery resolves global/project overrides and starts a fresh live result
window. Earlier requests can supply context; earlier results do not backfill it.
Restart resumes unchanged generations and durable progress. Definition or
admission changes cancel old-generation work and fence stale acceptance.

Live checks do not wait for the project backlog. Each session's first published
snapshot establishes its baseline, including an empty snapshot. Later source
generations can add results from sessions active when monitoring starts or updated
afterward. Startup admits activity from the last three minutes; processing delay
does not expire admitted work. Baseline and inactive-session updates advance delivery without entering
selection. Historical sessions require explicit isolated replay.

Deterministic publication wakes definition-based checks after it settles.
Built-in smart-check scheduling and historical enrollment are paused for this
milestone. Their implementations and saved preferences remain available for a
later migration.

The master Model-based Checks setting controls YAML definition discovery and
activation. The Jev connection supplies model credentials and usage accounting.
Partial fields, truncated text, and source gaps are frozen as explicit quality
facts; available evidence remains eligible. Missing expression values can make
one candidate not assessable, without blocking later session results. Core owns numbered
attempts and its known-safe transient retry. Unknown delivery does not authorize
a resend. SQLite persists selector progress, frozen candidates, claims, attempts,
answers, and status. Atomic acceptance can defer for ingestion catch-up and
recover a saved answer without another provider call.

Independent actions use the same frozen input. The last applied status write
wins in authored order. No-write retains the original status-write assessment,
time, and explanation. A branch success cannot clear another branch's failure.
The report uses eligible current evidence in the native 30-day cohort and counts
sessions before capping displayed targets. Snooze changes presentation, not
execution. Durable history supports recovery and provenance; the UI shows current
detail rather than an assessment-history browser.

Finite replay uses the same engine through an isolated `RunScope::Replay` and
Store. It settles each consumed trigger sequentially, preserves as-of-event
context, and bypasses live authored cooldown without bypassing evidence or
provider limits. It cannot mutate live rows. See the
[eval guide](../../apps/desktop/src-tauri/eval/README.md#session-replay) for diagnostics.

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
Use existing source adapters. File evidence comes from the existing parse pass;
native OpenCode uses the local journal page reader and its durable desktop
delivery queue. Projection passes do not also publish a legacy journal tail.

## Identity, order, and frozen evidence

A definition has a filename-stem ID, installation scope, and content revision.
An activation binds its effective definition, environment/session scope, run,
and durable generation. Session branches retain exact source ownership; a Git
branch name is metadata. A collection row groups by logical ID after resolving
project overrides; it does not merge branch status or evidence.

`ActivationScope` contains `environment`, `session_cwd`, and `run`.
`session_cwd` is the exact stored session working directory, not the resolved
repository root. `StoreCheckStore::activation_scope_key` serializes this scope
as JSON. Runtime records, SQL columns, and evidence target locators use
`activation_scope_key`. `EffectiveActivation.definition_scope` identifies the
global or project definition selected by discovery; `id.scope` identifies the
runtime isolation boundary. `EffectiveActivation.project_root` supplies the
resolved repository root for `repo.root`.

Session branch IDs encode the JSON tuple `[environment_key, agent, session_id]`.
The publication reader, persistence, and deletion share this encoding, so slashes
and escaped characters cannot merge distinct sessions. Session deletion removes
the branch's selector state and runtime rows before the same session key can
start again.

Internal `InstalledCheck*` names and the `installed:` display/snooze
namespace distinguish these rows from native detector IDs. The product term is
**Burn Checks**. These internal discriminators do not extend `DetectorId` or
authorize native remediation.

Native source order selects records. Publication cursors track delivery; engine
positions track committed observations. They are not interchangeable. The first
terminal result can trigger once. A correction keeps its logical result identity
and first-terminal position, but changes its evidence version. Duplicate delivery
is not a new attempt. Timestamps cannot replace proven source order.

`EffectiveActivation.bootstrap_observation_position` is the observation boundary
for a new generation. `bootstrap_publication` is its separate publication cursor.
The local evidence sink uses `tool_result_revision_key` to retain revisions; that
key is not a logical result identity.

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

Live checks do not wait for the project backlog. Non-journal sources use the
first published snapshot, including an empty one, as their baseline. Native
OpenCode captures the first page's materialized journal frontier and preserves
it across catch-up pages and restart. Historical results/corrections do not
trigger merely because discovery observes a newer generation. A pending result
can trigger when its first terminal position is after that baseline. Startup
admits sessions active in the last three minutes; processing delay does not
expire admitted work. Inactive-session delivery does not enter live selection.
Historical assessment requires explicit isolated replay.

Each journal page commits current rows, immutable outbox events, cursor,
baseline, diagnostics, and continuation together under the current lease/claim,
source incarnation, exact project, parser revision, and fork-admission fences.
Missing page membership is not removal. Real history gaps remain partial across
pages and restart; page limits alone do not create a gap. Prefix validation
permits later appends, while consumed rewrites or invalid cursors block for
explicit recovery without automatic rewind. Hashing and reconstruction still
scale with prior history. See [journal delivery](../session-coverage.md#opencode-journal-delivery).

Committed source publication wakes definition-based checks. Native journal
readiness and YAML report evidence do not depend on legacy metric freshness.
Built-in smart-check scheduling and historical enrollment are suspended. Their
saved results, preferences, and implementations remain.

The master **Model-based Checks** setting controls YAML definition discovery and
activation. **Decision model** exposes Jev only. The Jev connection supplies model
credentials and usage accounting.
Partial fields, truncated text, and source gaps are frozen as explicit quality
facts; available evidence remains eligible. Missing expression values can make
one candidate not assessable, without blocking later session results. Core owns numbered
attempts and its known-safe transient retry. Unknown delivery does not authorize
a resend. SQLite persists selector progress, frozen candidates, claims, attempts,
answers, and status. Atomic acceptance can defer for ingestion catch-up and
recover a saved answer without another provider call.

`AssessmentAcceptance` carries a claimed candidate, validated decisions, and
planned actions into `CheckStore::accept`. The returned `AcceptanceOutcome`
states whether the transaction accepted, deferred, or rejected the assessment.

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

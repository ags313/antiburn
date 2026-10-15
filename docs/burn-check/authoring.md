# Authoring Burn Checks

Use YAML to define one bounded assessment of recorded agent work. The
[specification](specification.md) gives the precise rules; the supported subset
below states what the runtime can execute.

> **Experimental draft: `v1-prealpha`.** Burn Checks are not yet standardized.
> Syntax, semantics, and capabilities may change, including breaking changes.
> There is no backward compatibility promise. Check the current draft before
> creating or changing a definition.

## Write one bounded check

Define one useful behavior, its required evidence, a bounded input, one narrow
question, and independent status actions. Include a positive case, an allowed
case, and an unclear case. Prefer a selected-work claim over an unsupported
whole-session claim. Missing activity does not prove that an action never
happened. Unknown evidence is different from a known empty collection.

Start with [Failed Retry Loop](examples/failed-retry-loop.yml). It includes
successful results so relevant recovery can end the failure pattern. Its
five-result window and three-attempt condition belong to that definition, not
the generic engine. An unrelated success does not reset the pattern. The check
can describe only the selected results and requests, not unobserved external
changes.

## Supported subset

The schema describes more than the runtime currently supports. The compiler
rejects unsupported definitions before activation, even behind `if: false`.
Do not discard unsupported fields or actions to make a definition run.

- **Trigger:** final `tool_result`, optionally filtered by normalized `tools`
  categories and result `status`. The first non-final-to-terminal transition
  triggers once; later terminal changes are corrections.
- **Window:** rolling message window, written as an integer or `{size, min}`.
  Minimum defaults to one; `min <= size`; maximum size is 40. New safely ordered
  events after activation enter the live window, including events with partial
  fields. Model cooldown defaults to
  `60s`; zero cooldown still uses single-flight execution.
- **Context:** `message`, eligible `window.messages`, `session.id`,
  `session.requests`, `agent.name`, and observed project fields such as
  `repo.root`. Field availability depends on source evidence. Missing values stay
  unavailable to expressions; available request text and tool fields remain
  usable when other context is partial. The model receives `message.quality`,
  `session.requests_complete`, and `session.source_coverage`. A required missing
  expression can make one candidate not assessable. It does not block later
  candidates from the same session.
- **Input and expressions:** JSON literals, nested fixed-key objects/arrays,
  whole `${{ ... }}` expressions, dotted paths, literal bracket keys/indexes,
  array `.length`, parentheses, `!`, `<`, `<=`, `>`, `>=`, `==`, `!=`, `&&`, and
  `||`. Conditions may be Boolean, bare expressions, or whole expressions.
  Type, phase, and availability rules still apply.
- **Decision:** exactly one block with one Choice question through
  `typesafe/jev-1.13.0`. Choice labels and probabilities are validated. Enable
  **Model-based Checks**, configure the Jev connection in
  **Settings → Checks**. Existing provider admission and resource limits apply.
- **Actions:** independent status actions with literal `success` or `failure`
  and an optional fixed explanation. The last valid applied write wins in
  authored order.
- **Presentation:** one logical filename-stem row in the native 30-day report
  cohort, with normal groups, detail, search, session navigation, and snoozes.
  Status and explanations do not grant native fix prompts, Auto Fix,
  verifier/watch support, burn estimates, or savings.

Unsupported capabilities include other triggers, model/mode/path filters,
instruction and skill inventories, measured activity, functions, projections,
maps, dynamic bracket expressions, embedded text templates, multiple
decisions/questions, Score, Noul, dynamic status results, decision-free local
checks, and message actions. The draft schema retains these forms; schema
acceptance is not runtime admission. See the [example support labels](README.md#examples).

Source support is limited to the native main-session contracts in
[session coverage](../session-coverage.md#burn-check-source-facts). Format names
alone do not prove support for every producer version, remote/WSL source, child
session, or retained history. In `session.requests`, user messages mean user-slot
content, including extension/API and synthetic input. They are not literal-human
proof and do not relax native detectors' approval gates.

## Identity and installation

The lowercase filename stem is the logical ID. It starts with a letter and
contains lowercase letters, digits, and hyphens. `name` is display text.

Discovery reads immediate `.yml` and `.yaml` files in `~/.antiburn/checks/` and
known native repository `.antiburn/checks/` directories. A nested session CWD
uses the deepest matching known repository root for definition lookup. Evidence
retains its exact environment/session CWD scope.

A project file replaces a global file with the same stem; files never merge.
Duplicate extensions in one scope invalidate that ID. An invalid or unreadable
override stays selected instead of authorizing fallback. Keep drafts outside
active folders. The app watches existing installation chains and reconciles
periodically; it never creates installation directories. Existing folder access
settings apply.

The Checks collection and search show one row per stem. Scoped activations and
branches remain separate. Internal display/snooze identity is separate from
native detector IDs; authors do not add detector enum values.

For the repository definition, follow the [dogfood setup](README.md#try-the-repository-dogfood-check).
The activation takes a fresh result boundary. Earlier user messages can supply
context, but historical tool results do not fill the live window. Restart resumes
unchanged work; changing the effective definition creates a new generation.

Live checks assess active sessions, not the historical backlog. A session's first
published snapshot is its baseline; new results from later source generations
enter the window for sessions active at startup or updated afterward. Startup
includes the last three minutes of activity; processing delay does not expire it.
The app does not wait for other sessions to finish indexing. Historical work
requires explicit isolated replay. Deterministic checks publish first and these
definitions run next. Built-in smart-check execution is paused for this
milestone; its implementation and saved preferences remain for a later
migration.

## Validate in layers

Examples use this editor modeline:

```yaml
# yaml-language-server: $schema=../schemas/burn-check.schema.json
```

When installing elsewhere, update that path or the editor's `yaml.schemas`
mapping. For `.antiburn/checks/failed-retry-loop.yml`, use
`../../docs/burn-check/schemas/burn-check.schema.json`. Do not add a YAML
`version` or `$schema` field.

Run from the repository root:

```sh
npm --prefix docs/burn-check ci
npm --prefix docs/burn-check test
npm --prefix docs/burn-check run validate
```

To validate a selected file:

```sh
node docs/burn-check/scripts/validate-checks.mjs .antiburn/checks/failed-retry-loop.yml
```

The validator checks restricted YAML 1.2 and Draft 2020-12 structure. It accepts
standard JSON-compatible tags and rejects duplicate/non-string keys, custom
tags, anchors, aliases, merge keys, multiple documents, and non-finite values.
It does not activate a check or call a provider.

| Layer             | Responsibility                                                                    |
| ----------------- | --------------------------------------------------------------------------------- |
| YAML parser       | One restricted JSON-compatible mapping                                            |
| JSON Schema       | Required fields, types, and unknown keys                                          |
| Runtime compiler  | Expressions, references, phases, window relationships, and supported capabilities |
| Source and engine | Order, completeness, frozen input, provider validation, and state transitions     |
| Desktop           | Current report rows, evidence details, navigation, search, and snoozes            |

Passing one layer does not prove the next. Developer replay is a separate,
finite diagnostic; see the [eval guide](../../apps/desktop/src-tauri/eval/README.md#session-replay).
It uses isolated state and does not alter live rows. It is independent of
repository dogfood work, not a desktop preview or automatic history run.

## Diagnose and interpret results

Read the normal **Not assessed** detail explanation. Invalid files also emit
`installed_check_invalid` ERROR logs with the path, diagnostic code, and available
line/column or configuration location. Repeated unchanged errors are deduplicated.
Diagnostics omit arbitrary authored values and selected work input.

Missing required context, result output, bindings, branch membership, or source
order blocks assessment without a provider call. Known gaps, truncation, or
relevant unfinished corrective work can make selected input incomplete.
Ambiguous meaning in otherwise complete evidence can yield an uncertain answer.
See [check coverage](../check-coverage.md#burn-check-definitions) for evidence limits.

A status message explains the result to the user; it is not a fix prompt or an
agent instruction. A no-write assessment preserves the earlier status, its
original explanation, and its status-write time. Another branch's success does
not clear a valid failure. Explicit success covers the assessed work only; it
does not prove repair, a clear whole session, or savings. Missing evidence,
uncertain answers, skipped actions, and execution errors do not create success.

A probability threshold is author policy, not an accuracy guarantee. Do not
fabricate probabilities or lower a gate to make a diagnostic pass. Snooze changes
presentation only; it does not pause evaluation or reset activation. Definitions
use existing usage accounting and resource limits, without a new dollar-cap promise.

## Data limits

Select only what the question needs. Arguments, results, and user messages can
contain private project data. A message window does not bound the separate
`session.requests` collection. Do not commit real transcripts, credentials, or
private captures as examples.

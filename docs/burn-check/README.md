# Burn Checks

Burn Checks assess recorded agent work from YAML definitions, with a model
decision or local status rules. The repository maintains six definitions for
retry loops, exploration, task scope, instruction comparisons, skill guidance,
and fast-mode trial advice. Each checks selected work, not whole-session quality.

> **Experimental draft: `v1-prealpha`.** Burn Checks are not yet standardized.
> Syntax, semantics, and capabilities may change, including breaking changes.
> There is no backward compatibility promise. Use the current
> [specification](specification.md) and [supported subset](authoring.md#supported-subset).

## Try the repository dogfood checks

The six canonical definitions live in `.antiburn/checks/`. See the
[definition table](#examples) for each file and its evidence limits. There is no
second maintained YAML copy in the documentation.

1. Enable this repository in Antiburn's native session discovery.
2. Open **Settings → Checks**, confirm **Model-based Checks** is on, choose a provider
   under **Decision model**, enter its required fields, and select **Save and
   use**. Jev is the recommended route. **Test connection** is optional; it sends
   synthetic input and can incur charges. See [decision connections](authoring.md#decision-connections)
   for compatible models and output requirements.
3. Continue normal work in a supported native main session. Discovery is
   automatic for supported active native main sessions. Fresh attachment can use
   bounded recent results and assess the latest ready window once. Later new
   terminal results with safe identity and order enter the live window, including
   usable partial evidence. It does not replay older windows.
4. Find a definition by its display name in Checks or local search. A check starts in
   **Not assessed**. On its first queued assessment, it intentionally moves to
   **Passed** as a provisional display result. This does not write a success
   assessment or increase the passed-session count. The card stays in **Failed**,
   **Passed**, or **Not assessed** while activity appears beside its result:
   **Passed · Checking**, **1 failed · Checking**, or
   **Passed · Waiting on 1 tool result**. Catch-up hides the countdown until
   delivery is ready. Open the detail pane for status messages, source details,
   and session cards.

**Model-based Checks** defaults to on when no preference was saved. An explicit
saved off stays off; malformed or unreadable storage keeps execution disabled.
YAML check cards and navigation appear only when a global or repository
definition is discovered. The Settings toggle and **Decision model** setup stay
available without definitions. A discovered check with no configured System One
provider shows a setup error in its detail pane and directs you to Settings.
Without a discovered definition, the app creates no placeholder check or error.

Failed retry loop needs at least three results in a rolling window of five. It
excludes relevant recovery, requested retries, and expected polling. None of the
definitions promises a finding in every session. All six require label
probabilities; a label-only Choice route cannot run them. Do not replace
probabilities with confidence or lower a gate to make a route fit. Failed retry
loop fails only when the model selects
`stuck` with a label probability of at least 0.95. Every other valid answer, including
`uncertain`, explicitly writes success and clears any earlier failure message.
All six definitions use complementary success and failure conditions with a 0.95
failure-label probability gate. This threshold is author policy, not an accuracy
guarantee or confidence in the underlying source facts.
Success means the failure condition did not match; it does not prove repair or
whole-session correctness. Missing data or a failed assessment does not write
success. Snooze hides active failure summaries;
it does not pause evaluation. Historical assessment requires explicit isolated
developer replay. Developer replay uses isolated state even when it reads a
canonical definition.

Each definition permits middle-text shortening only for
`activity[*].tool.result.content`. It preserves selected result records, linked
arguments, request/authorization text, and selected inventory scopes. Shortening is reported to the model
and in the check detail. If initial local request-size or preparation limits stop
preparation, the candidate is not assessed and no provider request is sent.
A preparation-limit stop does not prove that further permitted shortening could
not fit. Open the detail's **Open check file** action
to repair a definition in its default application. See
[fit large inputs](authoring.md#fit-large-inputs) for the limits.

YAML definition names do not identify native detectors. Each definition needs the
runtime and source capabilities listed below.

## Install your own check

Put an immediate `.yml` or `.yaml` file in `~/.antiburn/checks/` or a known native
repository's `.antiburn/checks/` directory. Create the directory yourself; the app
only watches and reconciles it. A project file replaces the global file with the
same lowercase filename stem. Definitions do not merge.

See [authoring](authoring.md) for identity, supported features, validation, and
diagnostics. Burn Checks use the existing Checks collection, detail pane,
search, and snoozes. They provide status and explanations, without native fix
prompts, Auto Fix, verification watches, burn estimates, or savings.

Settings selects one provider and model for all decision blocks in an assessment.
Do not put `model` in a YAML decision. It is an unknown field; the app does not
migrate or rewrite the file. Observed coding-agent model fields remain source
facts. A model check still needs a compatible connection.

Omit `decisions` for a local status check. It uses frozen context without model
discovery, warm-up, provider preparation, transport, inference, attempts, or usage
records. It still requires the master setting, a configured selected connection,
and the existing credential/authentication and runtime gates. Failed cached model
discovery alone does not block it. Local cooldown defaults to `0s`; model cooldown
defaults to `60s`. Local evidence is **Frozen local context**, not model input.
See [local status checks](authoring.md#local-status-checks).

Inputs can use pure functions, dynamic brackets, nested projections, embedded
text templates, and bounded maps with nested aliases. Status actions can compute
their result and explanation from frozen context and validated typed outputs.
These forms share an assessment evaluation budget. Omit decisions for local work,
or use independent named blocks with up to eight mixed Choice, Score, and Noul
questions per block. All blocks prepare before the first inference call and run
sequentially on one captured connection. Actions wait for the complete valid
answer set. Shared runtime work and retained-state limits bound the assessment;
there is no syntax-only block count limit. Tool-result filters can use recorded
model/mode/path facts under their source limits. Current global and repository
instruction and skill scopes freeze during candidate selection when a definition
references them. They do not prove past activation, effective session loading, or
historical non-use. Other triggers and message actions remain unsupported. See
[typed questions](authoring.md#typed-questions-and-actions),
[input transformations](authoring.md#transform-selected-input) and
[evaluation limits](authoring.md#shared-evaluation-limits).

Omitting `fit` is valid and does not produce a missing-fit warning. Input-size
explanations require an actual preparation or provider failure. The runtime
shortens only authorized fit targets and never adds summary calls.

## Examples

These six canonical dogfood definitions use the local schema modeline and the
validated runtime subset. Structural validity alone does not establish source
support. The linked coverage documents state the accepted sources and versions.

| Definition                                                              | Runtime support  | Assessed scope and limits                                                                                                                                                       |
| ----------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Failed retry loop](../../.antiburn/checks/failed-retry-loop.yml)       | Supported subset | Repeated unresolved failure in selected results, with recovery and requested-retry exclusions                                                                                   |
| [Over-exploring](../../.antiburn/checks/over-exploring.yml)             | Supported subset | Clearly unrelated completed reads/searches; plausible dependencies, tests, and verification remain allowed                                                                      |
| [Scope creep](../../.antiburn/checks/scope-creep.yml)                   | Supported subset | Supported completed work on a separate objective, with user approval kept separate from tool claims                                                                             |
| [Ignored instructions](../../.antiburn/checks/ignored-instructions.yml) | Supported subset | Current-file mismatch and recorded prior-correction conflict have separate labels and messages; partial scopes remain explicit                                                  |
| [Skill opportunities](../../.antiburn/checks/skill-opportunities.yml)   | Supported subset | Inventory-level next-step advice from observed configured descriptions, without a specific skill pick or historical non-use claim                                               |
| [Fast mode value](../../.antiburn/checks/fast-mode-value.yml)           | Supported subset | Standard-mode trial advice for explicitly non-urgent work; recorded speed, selected catalog mode, requested-tier preference, and name hints do not prove served tier or savings |

The instruction and skill checks select whole current scopes so absent arrays
stay unavailable rather than becoming empty. These inputs can include the other
inventory in that scope. The questions ignore unrelated entries, but the whole
scope must fit unchanged. See the [snapshot boundary and input limit](architecture.md#identity-order-and-frozen-evidence).

## Reference

- [Provider contracts](provider-contracts.md): pinned typed wire shapes and local
  criteria limits, separate from YAML support.
- [Authoring guide](authoring.md): installation, supported subset, and validation.
- [Specification: `v1-prealpha`](specification.md): precise current draft rules.
- [Schema](schemas/burn-check.schema.json): structural validation for the draft.
- [Expression parity corpus](tests/expression-parity.json): synthetic YAML with
  separate schema and compiler admission expectations, not runtime evidence proof.
- [Architecture](architecture.md): ownership and dataflow.
- [Session coverage](../session-coverage.md#burn-check-source-facts): accepted
  source shapes, producer pins, and evidence limits.
- [Check coverage](../check-coverage.md#burn-check-definitions): finding and result limits.

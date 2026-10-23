# Burn Checks

Burn Checks assess recorded agent work from YAML definitions, with a model
decision or local status rules. The maintained **Failed retry loop** check looks
for repeated attempts at the same unresolved
failure. It excludes relevant recovery, requested retries, and expected polling.

> **Experimental draft: `v1-prealpha`.** Burn Checks are not yet standardized.
> Syntax, semantics, and capabilities may change, including breaking changes.
> There is no backward compatibility promise. Use the current
> [specification](specification.md) and [supported subset](authoring.md#supported-subset).

## Try the repository dogfood check

Use the permanent repository definition at
`.antiburn/checks/failed-retry-loop.yml`. It follows the maintained
[Failed retry loop example](examples/failed-retry-loop.yml).

1. Enable this repository in Antiburn's native session discovery.
2. Open **Settings → Checks**, enable **Model-based Checks**, choose a provider
   under **Decision model**, enter its required fields, and select **Save and
   use**. Jev is the recommended route. **Test connection** is optional; it sends
   synthetic input and can incur charges. See [decision connections](authoring.md#decision-connections)
   for compatible models and output requirements.
3. Continue normal work in a supported native main session. Discovery is
   automatic for supported active native main sessions. Fresh attachment can use
   bounded recent results and assess the latest ready window once. Later new
   terminal results with safe identity and order enter the live window, including
   usable partial evidence. It does not replay older windows.
4. Find **Failed retry loop** in Checks or local search. A check starts in
   **Not assessed**. On its first queued assessment, it intentionally moves to
   **Passed** as a provisional display result. This does not write a success
   assessment or increase the passed-session count. The card stays in **Failed**,
   **Passed**, or **Not assessed** while activity appears beside its result:
   **Passed · Checking**, **1 failed · Checking**, or
   **Passed · Waiting on 1 tool result**. Catch-up hides the countdown until
   delivery is ready. Open the detail pane for status messages, source details,
   and session cards.

The example needs at least three results in a rolling window of five. It does
not promise a finding in every session. Its selected route must provide label
probabilities; a label-only Choice route cannot run this definition. Do not
replace probabilities with confidence or lower its gate to make a route fit.
It fails only when the model selects
`stuck` with a label probability of at least 0.95. Every other valid answer, including
`uncertain`, explicitly writes success and clears any earlier failure message.
All maintained examples use complementary success and failure conditions.
Success means the failure condition did not match; it does not prove repair or
whole-session correctness. Missing data or a failed assessment does not write
success. Snooze hides active failure summaries;
it does not pause evaluation. Historical assessment requires explicit isolated
developer replay. Dogfood work and developer evals are independent.

The definition permits middle-text shortening only for
`activity[*].tool.result.content`. It preserves selected result records, linked
arguments, and request/authorization text. Shortening is reported to the model
and in the check detail. If initial local request-size or preparation limits stop
preparation, the candidate is not assessed and no provider request is sent.
A preparation-limit stop does not prove that further permitted shortening could
not fit. Open the detail's **Open check file** action
to repair a definition in its default application. See
[fit large inputs](authoring.md#fit-large-inputs) for the limits.

YAML example names do not identify native detectors. Each definition needs the
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

Settings selects the provider and model for the supported single-Choice path.
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
their result and explanation from frozen context and validated Choice output.
These forms share an assessment evaluation budget. The runtime accepts no
decision or one Choice question. Multiple decisions/questions, Score, Noul, other
triggers, unsupported filters, inventories, and message actions remain
unsupported. See
[input transformations](authoring.md#transform-selected-input) and
[evaluation limits](authoring.md#shared-evaluation-limits).

## Examples

All six examples use the local schema modeline. Structural validity does not
establish runtime or source support. Unsupported examples identify missing
capabilities; they are not delivery promises.

| Example                                                   | Current runtime support | Scope or missing capability                                                           |
| --------------------------------------------------------- | ----------------------- | ------------------------------------------------------------------------------------- |
| [Failed retry loop](examples/failed-retry-loop.yml)       | Supported subset        | Repeated unresolved failure in selected tool results                                  |
| [Over-exploring](examples/over-exploring.yml)             | Supported subset        | Clearly unrelated read/search work in selected results                                |
| [Scope Creep](examples/scope-creep.yml)                   | Supported subset        | Possible separate objective in selected results, not a whole-turn judgment            |
| [Ignored Instructions](examples/ignored-instructions.yml) | Unsupported             | Needs instruction snapshots and message actions                                       |
| [Skill Opportunities](examples/skill-opportunities.yml)   | Unsupported             | Needs effective skill snapshots and message actions                                   |
| [Fast Mode Value](examples/fast-mode-value.yml)           | Unsupported             | Needs session-idle events, model/mode filters, measured activity, and message actions |

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

# System One provider contracts

These are the accepted wire contracts and local limits. They do not prove model
accuracy or make every wire feature available to YAML checks. See the
[supported subset](authoring.md#supported-subset) for production language support.

## Question types and criteria

| Selected route                                                     | Choice criteria | Score criteria | Returned details                                                                         | Contract source                                                                                                                                                                  |
| ------------------------------------------------------------------ | --------------- | -------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Jev `jev-1.13.0`                                                   | 1 to 26         | 1 to 10        | Choice/Score probabilities and confidence; Noul yes-probability                          | [Pinned model and shared validator](../../crates/antiburn-local/src/analysis/jev.rs), [default capabilities](../../crates/antiburn-local/src/analysis/jev/capabilities.rs)       |
| Cloudflare Clef or Clef-flash                                      | 2 to 26         | 2 to 10        | Choice/Score probabilities and confidence; Noul yes-probability                          | [Cloudflare adapter](../../apps/desktop/src-tauri/src/jev_cloudflare.rs), [checked-in metadata](../../apps/desktop/src-tauri/tests/fixtures/cloudflare/clef-model-metadata.json) |
| Custom direct or Cloudflare envelope                               | 2 to 26         | 2 to 10        | Strict adapter contract: Choice/Score probabilities and confidence; Noul yes-probability | [`typed_adapter`](../../crates/antiburn-local/src/analysis/jev/capabilities.rs)                                                                                                  |
| Ollama `0.40.1`, accepted `tev1`                                   | 2 to 24         | 2 to 24        | Choice/Score probabilities and confidence; Noul yes-probability                          | [Verified output support and renderer guards](../../apps/desktop/src-tauri/src/jev_ollama.rs)                                                                                    |
| Ollama `0.40.1`, accepted `clef` or characterized generic encoding | 2 to 26         | 2 to 26        | Choice/Score probabilities and confidence; Noul yes-probability                          | [Verified output support and renderer guards](../../apps/desktop/src-tauri/src/jev_ollama.rs)                                                                                    |

YAML requires at least two Choice or Score criteria. Use the intersection of that
language rule, the question's type-specific output interval, and any stricter
selected numeric limit. Do not apply Jev's Score maximum to an accepted Ollama
profile. Unknown runner, encoding, or renderer metadata does not inherit another
profile's support. Manual input limits do not establish output support.

The 26-criterion Choice limit is a local guard, not a universal upstream maximum.
Current upstream documentation alone does not widen the pinned Jev model or the
accepted runner contract. Ollama's source pin is
[`cf2a313a298066d572c36812e5ad30a21c0db13b`](https://github.com/ollama/ollama/tree/cf2a313a298066d572c36812e5ad30a21c0db13b/decision)
(`0.40.1`). The Tev1 renderer narrows its general 26-criterion interval to 24.

## Answer validation

A response must contain exactly the requested named answers with matching types.
Choice must return a declared label and a complete map of its declared labels.
Score must return a finite number from zero through the last criterion index.
Its probability and legend keys must be exactly `0` through `n-1`; each legend
value must equal the criterion at that index. Keys such as `01` are invalid.
Noul is a finite yes-probability in `[0, 1]`, not a Boolean, and has no separate
confidence field.

Every probability and available confidence value must be finite and in `[0, 1]`.
Probability sums use absolute error `<= 0.02` under `f64` arithmetic. Score sums
follow criterion index order, `0` through `n-1`, not lexicographic wire-key order.
Choice sums follow label-map order. Preserve the raw values; do not normalize
them, replace a valid Choice label, or recompute a Score from its distribution.
Provider confidence is distinct from a selected
label's probability.

## Request and accounting boundaries

One wire request can contain mixed question types. Each selected route also has
request-byte, response-byte, token, and question-count limits. Keep these limits
separate from verified output support. The local queue and transport memory caps
remain additional admission limits, not claims about provider capacity.

Usage accounting and answer validity are separate. A rejected answer can still
incur usage. Record unambiguous usage before answer validation. Ambiguous usage
must not select one duplicate value as the billed count. Neither a protocol pass
nor synthetic provider tests establish check quality or native source coverage.

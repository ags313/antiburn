# Claude plugin validation scope

The testing branch serves public **antiburn 0.2.1**, pinned to artifact commit
`24c6c009be9787ed4a62db035eda992e49aa31e3`. The artifact has 24 files.
Its automatic usage display requires no public configuration flag and ignores
the retired `usageUiEnabled` setting.

Acceptance uses Claude Code **2.1.303**. Results do not certify other host
versions, other operating systems, ordinary Claude chat, Cowork, managed
installations or other vendors. The desktop app and antiburn CLI are not
installation prerequisites.

## Evidence by artifact

| Artifact                                          | Recorded evidence                                                                                                                                                                                                      | Limits                                                                                                                             |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 0.2.0, `ac53288832217fd01b107c305f547fc09f264750` | Hosted installation, marketplace refresh, already-current update, retained-data removal/reinstall and exact installed-file hashes; native Terminal/Desktop Code commands and usage; resume and public/dev independence | The hosted update did not change versions. These observations certify that artifact and unchanged behavior, not every later patch. |
| 0.2.1, `24c6c009be9787ed4a62db035eda992e49aa31e3` | Exact installed-cache comparison: all 24 files match the pin; public 172, development 249 and packaging/shared 29 tests pass; strict public/catalog validation has no warnings or errors                               | Cache equality establishes installed bytes, not a real hosted version upgrade or fresh Desktop acceptance.                         |
| 0.2.1 Terminal                                    | Usage appears before the first explicit command; unavailable-state UI and help/refresh text are present; no retired unset reminder appears                                                                             | This record does not establish numeric usage, Desktop automatic display, an exact simultaneous strip count or zero inference cost. |

The 0.2.1 patch changes registration, configuration, metadata and tests; the
quota controller is unchanged from 0.2.0. Retain unaffected 0.2.0 quota and
lifecycle observations with their original scope. Do not label older evidence
invalid solely because it predates a later review, or combine test counts from
different snapshots as one run.

The earlier 0.2.0 native acceptance includes numeric usage and refresh,
help, clear/reload, public/dev co-loading in both orders and resume. Its
independent audit reports no installed-artifact mismatch. Zero model-cost
observations belong to those owned sessions; they do not automatically apply
to every later capture.

## Local integration and release gates

A local cached-Git 0.2.0 to 0.2.1 upgrade rehearsal is separate evidence from
hosted transport. Before catalog promotion, test an actual hosted upgrade from
a distinct older artifact, compare every installed file with the selected pin,
and record fresh Desktop automatic-display and numeric usage/refresh evidence
for the claimed scope. Keep already-current updates separate from upgrades.

Required automated checks include deterministic assembly, public capability
isolation, runtime tests, actual SDK type checks, focused format/lint checks,
strict validation, local lifecycle, secret scanning and repository quality
checks. Check the integrated revision's actual GitHub CI results. A reduced
draft-PR CI result does not establish release readiness. Reuse successful checks
until their relevant inputs change.

The recorded 0.2.1 quality scan has a development register-file size error.
Local preparation must resolve that gate and validate the changed development
source. A passing public runtime suite does not waive a development-source
quality failure. Keep current check results and captures under ignored local
review artifacts; do not put raw machine output or private session data here.

## Capability and distribution boundary

Public files physically exclude antiburn CLI discovery/access, live checks and
debug previews. Local development keeps checks and actions with burn animation
disabled. Host account authorization and usage access remain host managed.
Native commands need an enabled mod runtime; a loose owned findings skill can
intercept the command, so follow the targeted retirement instructions in the
[migration guide](migration.md#retire-the-owned-standalone-skill).

Codex, Cursor, OpenCode and Pi remain unpublished candidates. Their adapters and
synthetic tests do not establish native host compatibility or cross-vendor quota
support.

The testing-branch catalog is distinct from main promotion. Merging the catalog
into main distributes its pinned plugin without a GitHub Release or tag.
Every push, PR readiness change and catalog promotion needs the user's explicit
approval for that action. Release drafts, tags and publication need separate
authorization. See [distribution](distribution.md).

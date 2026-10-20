# Claude plugin validation scope

The testing branch serves public **antiburn 0.2.1**, pinned to artifact commit
`24c6c009be9787ed4a62db035eda992e49aa31e3`. The artifact has 24 files.
Its automatic usage display requires no public configuration flag and ignores
the retired `usageUiEnabled` setting.

Historical acceptance used Claude Code **2.1.303**. The focused 20 October 2026
pass used Terminal **2.1.304** and Claude Desktop **2.42304.2** on macOS. Results
do not certify other host versions, other operating systems, ordinary Claude
chat, Cowork, managed installations or other vendors. The desktop app and antiburn CLI are not
installation prerequisites.

## Evidence by artifact

| Artifact                                          | Recorded evidence                                                                                                                                                                                                      | Limits                                                                                                                             |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 0.2.0, `ac53288832217fd01b107c305f547fc09f264750` | Hosted installation, marketplace refresh, already-current update, retained-data removal/reinstall and exact installed-file hashes; native Terminal/Desktop Code commands and usage; resume and public/dev independence | The hosted update did not change versions. These observations certify that artifact and unchanged behavior, not every later patch. |
| 0.2.1, `24c6c009be9787ed4a62db035eda992e49aa31e3` | Exact installed-cache comparison: all 24 files match the pin; public 172, development 249 and packaging/shared 29 tests pass; strict public/catalog validation has no warnings or errors                               | Cache equality establishes installed bytes; fresh Desktop acceptance is recorded below.                                            |
| 0.2.1 Terminal                                    | Usage appears before the first explicit command; unavailable-state UI and help/refresh text are present; no retired unset reminder appears                                                                             | This record does not establish numeric usage, Desktop automatic display, an exact simultaneous strip count or zero inference cost. |

## Focused 0.2.1 acceptance — 20 October 2026

All 24 installed public files match the pinned artifact and integrated candidate
`257ee35006f5ea939be3e3b5a1a9009bd07fe1a7`. Terminal displayed numeric usage
before any command. A fresh Desktop session initialized with the host
`/reload-plugins` command displayed the strip without an antiburn command.
Help, numeric usage and explicit refresh passed on both surfaces. Terminal
reported $0.0000, zero input/output/cache tokens and zero API duration; all four
owned transcripts contain zero assistant records. The
[plugin page](../../plugins/README.md) includes user-provided animated strip-and-prompt examples for Desktop Code
and Terminal, covering synthetic usage states and hover details. These are
presentation captures, separate from the command and refresh acceptance evidence.

The local development artifact matched all 39 generated files. Native Desktop
checks covered labeled synthetic numeric and retry states, captions-above
layout, retry-to-checking interaction, sample-check expansion and Ignore
reducing five checks to four. The temporary local installation and its local
marketplace declaration were removed afterward; the public installation stayed
at 0.2.1. Synthetic examples do not certify public account failure behavior.

The pass inspected dark full and split/reduced-width layouts. It did not repeat
the full light/dark/reduced-motion matrix or measure the smallest Desktop
fallback breakpoint. The Desktop blank new-session landing page is outside the
initialized mod runtime. An initial plugin command during initialization can
show “isn't a command here” despite executing successfully. Its cause is not
isolated. The warning cleared after a later recognized public command; the
host-only startup path did not show it. Do not label this run warning-free.

The historical rows above retain their original evidence limits. This focused
pass supplies fresh numeric, Desktop and inference-cost observations; it
does not add an older-version upgrade requirement for this first rollout.

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

## Desktop startup and visual follow-up — 20 October 2026

A fresh public session reproduced the startup warning with `/antiburn help` as
its first action. Help returned successfully while the composer said the
command was unavailable. After initialization, the command picker listed
`antiburn` with its registered description. A second help invocation succeeded
and cleared the warning. This supports a Desktop command-discovery/startup
timing mismatch; host internals and a corrective change remain unverified.
The development registration order differs and has also shown the warning, so
moving public registration alone is not an established fix.

The public numeric strip remained readable in light mode with Claude's Reduced
animation preference selected, at full width and a 21% chat split share. The
narrow split stacked both meters without visible overlap. Dark mode with the
original System animation setting also passed that stacked layout check.
At a 10% share, the host clipped the entire chat underneath the terminal;
this does not certify the plugin's smallest fallback breakpoint. Selecting
Claude's Reduced preference does not establish the SVG operating-system
reduced-motion media query or complete the animated fixture matrix. Original
System theme and animation settings were restored. macOS Reduce Motion was
inspected and was off; the system preference was not changed. Existing passing
runtime tests cover 12-column fallback readings, 23-column accessible percentage
labels on both surfaces, and Desktop geometry at 24, 40, 80 and 120 columns.
Graphics tests assert the reduced-motion CSS rules for checking dots and hover
details; development fixture tests assert that warning pulses stop under reduced
motion. These are automated checks, not native visual certification of those
states. Public runtime bytes remain unchanged.

## Development terminal pane follow-up — 20 October 2026

The initial demo used the classic renderer because its isolated launch excluded
user settings. Starting the same development artifact with the process-local
setting `{"tui":"fullscreen"}` restored right docking. The development guide now
uses that launch setting without changing user defaults. Docking requires at
least 110 terminal columns; narrower surfaces use the inline fallback.

A fresh classic-layout check in Claude Desktop's embedded shell showed working
keyboard interaction: Tab highlighted buttons and scrolled hidden controls into
view, Enter switched between usage and checks modes, and the usage sequence
advanced through synthetic examples, including the near-limit state. A mouse
click on the same mode button did not invoke its action. This narrows the
observation to mouse interaction in that tested host path; it does not isolate
the host cause or establish a fix. The debug controls use native SDK Buttons.
No public runtime bytes changed. The development debug pane is excluded from
the public package.

## Local integration and release gates

This is the first public rollout; there are no users on older plugin versions.
An older-version upgrade path, migration implementation and hosted upgrade
rehearsal are not release requirements. Historical update checks remain test
records only. Validate fresh installation and the exact pinned files, along
with removal, reinstall and public/development independence. Verify the
permanent main marketplace route after the approved merge.

Required automated checks include deterministic assembly, public capability
isolation, runtime tests, actual SDK type checks, focused format/lint checks,
strict validation, local lifecycle, secret scanning and repository quality
checks. Check the integrated revision's actual GitHub CI results. A reduced
draft-PR CI result does not establish release readiness. Reuse successful checks
until their relevant inputs change.

The earlier 0.2.1 quality scan reported a development register-file size error.
The integrated candidate extracts development views and passes the quality scan
with a score of 100 and zero errors. Its development SDK check, 249 runtime
tests and independent TypeScript review passed. Keep current check results and captures under ignored local
review artifacts; do not put raw machine output or private session data here.

## Capability and distribution boundary

Public files physically exclude antiburn CLI discovery/access, live checks and
debug previews. Local development keeps checks and actions with burn animation
disabled. Host account authorization and usage access remain host managed.
Native commands need an enabled mod runtime. See the
[local lifecycle guide](installation.md#claude-local-lifecycle-validation) for
installation and isolation checks.

Codex, Cursor, OpenCode and Pi remain unpublished candidates. Their adapters and
synthetic tests do not establish native host compatibility or cross-vendor quota
support.

The testing-branch catalog is distinct from main promotion. Merging the catalog
into main distributes its pinned plugin without a GitHub Release or tag.
Every push, PR readiness change and catalog promotion needs the user's explicit
approval for that action. Release drafts, tags and publication need separate
authorization. See [distribution](distribution.md).

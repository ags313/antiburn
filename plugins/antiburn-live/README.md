> The local development artifact is **antiburn-dev**, separate from the public
> **antiburn** candidate. Public output excludes CLI discovery, live checks and
> previews. This guide documents the development feature set; see
> [the plugin page](../README.md) for current distribution status.

# antiburn development source

Burn animation and its preview controls are temporarily disabled until refined.
Live checks, actions, static branding, quota controls and loading indicators
remain available. Animation algorithms remain in the source for later work.

This folder contains the maintained development implementation, including
experimental checks and debug tools. The public usage-only extension is
[`antiburn`](../antiburn/README.md). Its generated folder
physically excludes this development implementation. The cross-vendor core
keeps the `antiburn` name.

The command examples below describe this source's `/antiburn-dev` registration.
Assembly rewrites commands and state owners to `antiburn-dev` in the
local development artifact, so it can coexist with the release and core.

A Claude Code mod that shows account usage above the prompt. Usage is the
default mode. Set `ANTIBURN_LIVE_MODE=checks` before launching Claude Code to
add the existing token-burn checks and their fixes. Other values select usage.
Changing the flag requires a reload or restart; `/clear` retains the selected
mode. The mod makes no network calls.

In this README, "desktop" means the Claude desktop app. The mod does not
connect to the antiburn app (`apps/desktop`). See
"Privacy and resources" below.

## Install and run

Generate the local-only development artifact from the repository root:

```bash
node scripts/assemble-claude-mod.mjs --dev
claude --plugin-dir .agent-artifacts/claude-mod/dev/plugins/antiburn-dev
```

Regenerate after changing source files. The host can reload changes in the
emitted folder; it does not run the assembler for you. Use a separate test
session and normally enable only one usage strip per session.

The generated marketplace at `.agent-artifacts/claude-mod/dev` is named
`antiburn-local-dev`; it lists only `antiburn-dev`. Keep it local.
The existing testing marketplace still points at the old mod artifact; the unified
candidate is local and unpublished. It contains no
longer advertises the old `antiburn-live` identity or an unverified release tag.
See the public README for release-candidate status and remaining gates.

The source manifest retains its development name. Do not publish this folder.
Its hooks module is `hooks/register.tsx`.

## Surfaces

### Persistent usage

The small `a` mark accompanies the “5-hour” and “Weekly” meters. Usage stays
closest to the composer, below optional checks, including while Claude works.
A host survey temporarily owns the band. Claude Code scrolls a tree taller
than its `maxRows`; the mod does not override that height limit.

The Usage card in `/antiburn-dev debug` holds usage details and a Refresh button
for the native snapshot. Details include the exact percentage, reset timestamp
and recognized additional windows. Values come from `session.usage()` and `session.measure`, as last
reported by Claude Code. There is no independent provider fetch or freshness
claim. Desktop engine 2.1.295 was observed returning only a weekly window
from `session.usage()` after the five-hour reset; the mod preserves that missing
percentage as unknown and uses “Starts when a message is sent” for its reset
text when a valid weekly reading is present. This copy is a presentation
fallback; the SDK does not expose an inactive-window flag. Empty readings say “Usage not available yet”; missing windows use an
unknown value, not zero. Failed refreshes label retained values “Last known” in the debug pane and accessible meter descriptions. Desktop keeps the same strip height across ready, empty, and failed readings.
An exceeded percentage stays exact while its meter saturates at 100%.

The pace marker uses the elapsed fraction of the known five-hour or weekly
period. It is a continuous neutral line before usage reaches that unrounded fraction,
then a continuous orange line. High usage remains static orange.

Desktop draws the labels, exact percentages and whole resting meter dots in SVG.
The filled count rounds to the nearest dot, with one lit for nonzero usage and
all dots lit only at 100%. Desktop fill transitions can pass through partial dots.
Terminal uses whole filled or empty glyphs. Red warning dots stay fully to the
right of a reached pace marker; the marker never splits a dot's color.
Bundled Bitcount glyphs provide dot typography without loading a font at runtime.
Desktop primary captions use 14px lettering; dim secondary copy stays at 12px.
The usage-strip mark is 13px wide, including activity previews.
The small mark centers in its unchanged 14px slot against the complete meter block. A 5px leading inset
balances the host padding against the 13px gap before the meters. A matching
5px trailing inset gives the last dot the same band-edge clearance as the mark.
Reset details occupy reserved caption space: hover reveals them after a 300ms dwell; focus reveals them immediately without
adding rows. Click away or move focus to dismiss. Reset text centers vertically
against the label. The 18px pace marker centers on the dots and uses one continuous stroke.
Pace hover contrast uses the same 300ms dwell; focus and tap stay immediate.
Missing percentage readings display `—`; internal availability and read-error states remain distinct.
Desktop draws no native tooltip; the Usage card in `/antiburn-dev debug` keeps full
reset and pace text available, and accessible labels retain the same information.

Desktop presentation includes the once-per-session entrance, 600–960ms fill
sweep, changed-digit rolls and reset hover transitions. SVG CSS handles motion;
host events release expired presentation bookkeeping without scheduling timers. New readings interrupt entrance and
settle to current values. Resize, surveys, clear and teardown cancel presentation
work. Same-session reload does not grant another entrance. CSS reduced-motion
rules remove movement and delayed fill and use 80ms opacity for entrance/details.
Live reduced-motion preference changes were verified on desktop 2.37337.1.
Full native boundary-frame validation is tracked separately;
these implementation rules are not a claim of pixel-identical native output.

Terminal retains an explicitly accepted static fallback: native text captions,
whole/half-dot glyph approximations, exact numeric values and a two-cell brand
gap. It schedules no presentation timers and does not assume pixel Image
support, pixel cell dimensions, or an unavailable reduced-motion signal. Both
adapters reduce dot counts as space shrinks. The desktop's 80-column threshold
uses the tested 8px-per-column scale; other host scales still need validation.

### Optional checks strip (`ANTIBURN_LIVE_MODE=checks`)

The strip shows when at least one check is active. It names the top active
check (the first in catalog order: X1, D, O, T, E1, K1, C1, K2) and counts the
active checks as `N to fix`. While Claude works, or while a survey shows, the
strip gives its place back to Claude Code.

| Control | Label                                                        | Terminal hotkey | Action                                                              |
| ------- | ------------------------------------------------------------ | --------------- | ------------------------------------------------------------------- |
| Fix     | The fix label of the top check, such as `Ask Claude to fix`  | `1`             | Runs the fix of the top check.                                      |
| View    | `View checks` (`View` below 80 columns)                      | `2`             | Opens the checks pane and tries to move the focus to its first row. |
| Dismiss | `×`: a close glyph on the desktop, a dim `×` in the terminal | none            | Hides the strip until a new check arrives.                          |

In the terminal, the labels start with the digit (`1 Ask Claude to fix`,
`2 View checks`). The desktop prompt sends a digit to Claude, so the desktop
strip has no hotkeys and no digits.

The checks strip has four forms. Each keeps the top check name because its
fix button acts on that check. The persistent usage row owns the brand mark.

| Width (columns) | Form                                                                                                  |
| --------------- | ----------------------------------------------------------------------------------------------------- |
| 100 and more    | Count with `(N new)`, check name and its metric.                                                      |
| 80 to 99        | The metric goes. `(N new)` stays.                                                                     |
| 56 to 79        | The `View` label is shorter. The desktop drops `(N new)`. The terminal keeps `(N new)` while it fits. |
| Less than 56    | The buttons move to a second row.                                                                     |

The terminal also moves the buttons to a second row when the check name does
not fit beside them. It shows the count in a box only while the check name
still fits.

After a dismiss, the strip stays hidden until a check becomes new: a check
appears for the first time, comes back after it stopped, or reaches a higher
retry step. The strip then shows `(N new)` beside the count. The same check
in a later turn does not bring the strip back.

When a check resolves and no check is active or watched, the strip shows
`✓ <Check name> resolved` for one turn (`+N` counts the other checks that
resolved). A watched check does not show in the strip.

### Footer chip

The chip is in the session mode footer, after any mode labels. Press it to
open the checks pane.

| Chip             | Meaning                                                       |
| ---------------- | ------------------------------------------------------------- |
| `antiburn · N ●` | N checks are active.                                          |
| `antiburn · N ⧗` | No check is active. N checks are watched after a fix.         |
| `antiburn · N`   | No check is active or watched. N checks still build evidence. |
| `antiburn ✓`     | No open check.                                                |

The chip does not count a check that you ignore. On a VS Code or mobile
surface, which draws no strip, the mod puts the same text in the status line
while a check is active or watched. The terminal gets no status line text.

### Checks pane

The pane opens with the title `antiburn` and asks for 48 columns and 24 rows.
These actions open it in checks mode: `/antiburn-dev checks`, the demo commands, the chip, the View
button, and the first press of a send fix in the strip when it needs a
confirm.

A new active check also opens the pane, without the keyboard, so the next
prompt still goes to the composer. The mod does not open it when it is already
open, or after the person closes it by hand in this session (`/clear` resets
this). The host draws a pane that opens this way only from 144 terminal columns
(110 if the person opened it before). Below that, the pane waits undrawn, and
the strip and the chip still show the check.

In the terminal, the host docks the pane on the right only in the fullscreen
layout (`/tui fullscreen`, or `"tui": "fullscreen"` in the user settings). In
the classic layout, the pane shows inline below the output. The launch flag
`--setting-sources project,local` skips the user settings, so it also skips that
setting.

- **Heading.** `Checks` with the count of active checks.
- **Groups.** Active checks show first. `Awaiting verification` holds the
  watched checks. `✓ Resolved` (`Resolved` on the desktop) is folded. To open
  it, press its heading in the terminal or its chevron on the desktop. The pane leaves out checks that build
  evidence, checks that are quiet, and checks that you ignore.
- **Cards.** Each card shows the check name, its metric and the first sentence
  of its reason. Only one card is open at a time. An open card shows the full
  reason, a `Cause` line when the check has one, the fix block, and the
  actions. A watched card shows `Checking the next turns.` in place of the
  actions. A resolved card shows `✓ Fixed` or `✓ Stopped by itself`.
- **Fix block.** A fill fix that is one command shows one line, such as
  `Fills /model claude-opus-5. You send it.` The K1 advice shows
  `Next time, start with /<skill>. It may take fewer steps.` Other fixes show
  their text under `Fix prompt`, `Prompt` or `Compact instructions`. Text
  longer than about 180 characters shows `Show all` and `Show less`.
- **Empty pane.** Before the first check: `No checks yet`. After a check showed
  and none is active: `Nothing to fix right now.`
- **Footer.** A link: `Get more from every session with antiburn app ↗`, to
  https://antiburn.com. While the demo tour runs, the footer also shows
  `Demo tour · step N of 4` and a `Next step` or `End tour` button.

|              | Terminal                                                                                                                                                    | Desktop                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Rows         | A coloured `●` and the check name, a metric line, the reason.                                                                                               | A card with a tinted icon tile. Metrics and skill names draw as SVG.                 |
| Open a card  | The `▸` button at the right (`▾` when open). The first row has the focus when the pane opens, so Enter opens it. The pane scrolls the open card to the top. | The chevron at the top right of the card (`Show more`, `Show less`).                 |
| Card actions | `f <fix label>` and `h Ignore this session`. The `f` and `h` keys work on the open card.                                                                    | The fix button. `Ignore this session` shows on the open card. Cards have no hotkeys. |
| Counts       | Dim numbers.                                                                                                                                                | Round badges beside each section heading.                                            |
| Background   | none                                                                                                                                                        | The antiburn mark, behind an empty pane or under the cards.                          |

## Commands

The mod registers `/antiburn-dev`. The separate usage pane is currently disabled;
usage readings, developer examples and animation inspection are in the Usage
card of `/antiburn-dev debug`. The Checks card groups mode controls and check demos. Only checks mode advertises and accepts the
`checks` and `demo` arguments. Usage mode does not collect checks, discover
skills, publish check status, open check panes or run check actions.

| Command                   | Action                                                                                                                                                                                                                                 |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/antiburn-dev`           | Opens live checks in checks mode. In usage mode, confirms that limits are shown above the prompt without opening a pane.                                                                                                               |
| `/antiburn-dev checks`    | Opens the checks pane in checks mode.                                                                                                                                                                                                  |
| `/antiburn-dev demo`      | Shows a sample view with all eight checks and opens the pane. Five are active (E1, D, T, K1, K2), one is watched (X1) and two are resolved (O by its fix, C1 by itself). The sample stays until `demo off`. `demo states` is an alias. |
| `/antiburn-dev demo tour` | Starts a tour of 4 steps on an empty, separate set of checks, and opens the pane.                                                                                                                                                      |
| `/antiburn-dev demo next` | Plays the next tour step as one turn. The `Next step` button in the pane does the same.                                                                                                                                                |
| `/antiburn-dev demo off`  | Ends the sample view or the tour, and shows the live checks again.                                                                                                                                                                     |

Any unrecognized argument replies with the usage line.

The tour steps are:

1. A retry loop is the first warning (X1).
2. Three more checks arrive (T, E1, K2). The tour dismisses the strip first,
   so the strip comes back with `(N new)`.
3. The context gets heavy, and the retry loop stops (D with its cause, X1
   resolves).
4. A skill goes unused, and edits leave the plan (K1, C1).

The tour does not show O, because O fires only on the first turn. The tour
uses its own set of checks. The real hooks keep feeding the live checks while
a demo shows.

**Caution:** a fix in a demo or a tour acts for real. It sends the prompt to
Claude, fills the prompt box, or compacts the session.

## Checks

The thresholds come from `CATALOG.live` and `CATALOG.policy` in
`hooks/catalog.ts`. Only the main thread counts, except for X1. A check is
tested at the end of each main turn.

| ID  | Name              | Fires when                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Fix                                                                                                                                                                                                                                                 |
| --- | ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| X1  | Retry loop        | The same tool call fails 3 or more times in one turn of the main thread or of a subagent. A success of the same call resets its count. The call matches on its input, without the `description`, `tool_use_id`, `agentId` and `consent` fields. Metric: `3 tries · Bash`, with `· subagent` for a subagent. The check comes back as new at 5, 10 and 20 tries.                                                                                                                                                                                                                                                         | `Ask Claude to fix`: sends a prompt that asks Claude to stop the retry, name the cause and change the approach.                                                                                                                                     |
| D   | Context heavy     | One request in the turn carries more than 400,000 tokens (input, cache read and cache creation), or two context readings in a row are at 90% or more. A context reading between turns also updates it. Metric: `Nk tokens per request` or `N% of context used`. When file reads are 50% or more of the context (an estimate at 4 characters per token, since the last compaction), the card adds `Cause · File reads are about N% of it (Nk tokens).`                                                                                                                                                                  | `Compact now`: compacts the session with instructions to keep the task, the decisions, the open questions and the files in progress. With a file-read cause, the instructions also keep the findings and file paths and drop the raw file contents. |
| O   | Obsolete model    | At the end of the first main turn, the main model has a newer replacement in `policy.modelReplacements` (for example `claude-opus-4-8` to `claude-opus-5`). The model key ignores a provider prefix, a date suffix and case. After the first turn, an open O goes quiet. Metric: `opus-4-8 → opus-5`.                                                                                                                                                                                                                                                                                                                  | `Switch model`: fills `/model <replacement>` in the prompt box.                                                                                                                                                                                     |
| T   | Overthinking      | After 3 or more main turns, at least 5 main requests use an effort above the cap (`max`, `ultra` or `xhigh`), these are at least 50% of the requests that name an effort, and the latest request is still above the cap. A numeric effort does not count. Until then, T builds evidence. Metric: `6 of 8 requests at max`.                                                                                                                                                                                                                                                                                             | `Lower effort`: fills `/effort high`.                                                                                                                                                                                                               |
| E1  | Reading spiral    | In one main turn, Claude makes 20 or more successful read calls over 10 or more files or folders, and no successful edit. Reads are `Read`, `Grep`, `Glob`, `LS`, `NotebookRead`, and a `Bash` call that Claude Code marks read-only or whose every part runs a read program (`cat`, `head`, `tail`, `less`, `nl`, `wc`, `ls`, `tree`, `find`, `fd`, `rg`, `grep`, `ag`, or `sed -n` without `-i`) with no redirect into a file. Metric: `24 reads · 0 edits · 17 files`.                                                                                                                                              | `Ask Claude to focus`: sends a prompt that asks Claude to state its findings, its hypothesis and the files it will change, then change them or ask one question.                                                                                    |
| K1  | Skill not used    | The first main turn after your request takes 10 or more steps, the request does not start with `/`, and no skill or slash command ran for it. The mod then reads the list of installed commands (without the built-in commands and `/antiburn-dev`) and looks for a match: the request names the skill as `/name`, names a dashed skill name in words, or shares 3 or more keywords with the skill description. Metric: `/name fits · N steps`. When the request names `/name` after its first word, the metric is `/name named · not run`, and the reason says that a skill name runs only at the start of a message. | `Got it`: advice only. It sends nothing and sets the check quiet. The card says `Next time, start with /name.`                                                                                                                                      |
| C1  | Outside the plan  | After you approve a plan (`ExitPlanMode`), Claude edits 3 or more files whose names are not in the plan text or in your requests from that point. Test files do not count. Metric: `N files outside the plan`.                                                                                                                                                                                                                                                                                                                                                                                                         | `Ask about extras`: fills a prompt that asks Claude to list each change outside the plan with its reason, and to keep only the ones you approve.                                                                                                    |
| K2  | Repeated workflow | The same 3 or more shell commands, in the same order, run in 3 or more main turns. A command is named by its program and first word, such as `cargo test` or `npm run build`. `cd`, `echo` and read programs do not count. Metric: `same 3 commands × 3`.                                                                                                                                                                                                                                                                                                                                                              | `Draft a skill`: fills a prompt that asks Claude to write `.claude/skills/<name>/SKILL.md` for these commands and to show it before it saves it.                                                                                                    |

Before K1 matches, the mod removes the context blocks that Claude Code adds to
a message, such as `<system-reminder>`. A machine prompt, such as a loop
wake-up, is not a request.

## Check lifecycle

| Stage    | Meaning                                                                        | Shown in                                                 |
| -------- | ------------------------------------------------------------------------------ | -------------------------------------------------------- |
| building | The check collects evidence (T only).                                          | Nowhere.                                                 |
| active   | The check fired and offers its fix.                                            | Strip, chip, pane.                                       |
| watching | The fix ran. The mod checks the next turns.                                    | Chip (`⧗`), `Awaiting verification`.                     |
| resolved | The pattern stopped.                                                           | `✓ Resolved` group, and the resolved strip for one turn. |
| quiet    | The check no longer applies to the current task. A later match shows it again. | Nowhere.                                                 |

A fix moves a check from active to watching:

- **Send and compact fixes** (X1, E1, D) move to watching when the fix runs.
- **Fill fixes** move to watching only when a later request shows the change.
  O moves when a request uses a model with no replacement. T moves when a
  request uses an effort at or below the cap. C1 moves when you send the
  filled prompt unchanged. K2 moves when you send its filled prompt unchanged.

A watched check resolves when its rule clears. Its card then shows
`✓ Fixed`. A check that clears without a fix shows `✓ Stopped by itself`.
If the rule still fires for 3 more turns, the check goes back to active.

Each check clears on its own rule:

| ID  | Resolves when                                                      |
| --- | ------------------------------------------------------------------ |
| X1  | A main turn ends with no call that failed 3 times.                 |
| D   | The main session compacts, or the context falls below both limits. |
| O   | A request uses a model that has no replacement.                    |
| T   | The latest request uses an effort at or below the cap.             |
| E1  | A main turn edits a file.                                          |
| K1  | A later request runs the skill.                                    |
| C1  | 3 turns in a row add no file outside the plan.                     |
| K2  | Claude writes a `skills/<name>/SKILL.md` file.                     |

Some checks go quiet: O after the first turn, K1 after `Got it` or a new
request, and C1 after a new request.

## Fix actions and safety

| Kind    | Checks       | What the press does                                           |
| ------- | ------------ | ------------------------------------------------------------- |
| Send    | X1, E1       | Sends the fix prompt to Claude as your message.               |
| Compact | D            | Compacts the session with the compact instructions.           |
| Fill    | O, T, C1, K2 | Replaces the text in the prompt box. You read it and send it. |
| Advice  | K1           | Sends nothing.                                                |

The mod sends to Claude only the fix text from the catalog. It fills in the
replacement model (O), the skill name (K1) or the command names (K2).

**Two-press confirm.**

- A send fix needs two presses when the permission mode is `auto` or
  `bypassPermissions`. It also needs two presses before the mod has seen the
  first permission mode. On a host that an organization manages, Claude Code
  does not run the classic hooks for this mod, so the mod never sees the
  permission mode and every send fix needs two presses. The first press opens the card on its prompt and
  changes the label to `Confirm send`. A first press in the strip also opens
  the pane. The second press sends.
- A compact fix on a card always needs two presses, because a compaction
  cannot be undone. The label changes to `Confirm compact`. The
  `Compact now` button in the strip compacts on one press.
- The armed button takes the primary style. A new prompt, a press on
  `Ignore this session`, or a press that opens or closes a card or a group
  cancels an armed fix.

**Compact.** If the session has too few messages to compact, the mod logs
`Compaction did not run. This session has too few messages to compact yet.`
and the check stays active.

**Ignore this session.** This button on an open active card hides the check
for the rest of the session. The check leaves the strip, the chip count and
the pane. `/clear` or a new session shows all checks again.

**Dismiss.** The `×` button hides the strip only. The chip and the pane still
show the checks.

**Fail open.** Each hook that can block an action has a `.catch` that runs the
rest of the chain. An error in the mod does not block a tool call, a command
or a turn.

## Privacy and resources

- **No network, processes, model calls or polling.** The hooks module does not
  call `$.http`, `$.process` or `$.model`. It reads `$.clock.now` to calculate
  reset and pace labels on redraw; no countdown polling runs. CI permits `clock.now` and one-shot `clock.after` for the explicit finite debug sequence; it rejects other clock calls in the validator call list. Terminal draws statically.
  The footer link to https://antiburn.com is a plain link. The mod does not
  fetch it.
- **The host runs the model, and only after a press.** A fix with a prompt
  calls `$.prompt.submit`, and a compact fix calls `$.session.compact`. Both
  ask Claude Code to act. The mod never starts a model call by itself.
- **No files.** The mod writes no file and does not use `$.store`. A test
  checks that a check, its fix and its resolve make no `fs.write`. CI does not
  check `$.store` or `$.fs`. All state ends with the session.
- **No link to the antiburn app.** The mod and the antiburn app share no
  socket, file or call. The app reads the session transcripts that Claude Code
  writes to disk. It does not read the mod state. The two share one thing: the
  policy values in `hooks/catalog.ts` and in `ReportCatalogs` (see
  [Development](#development)).
- **What it keeps.** Usage readings and interaction state live in
  `antiburn-live.usage`. In checks mode, the check counts live in memory in `LiveChecks`
  (`hooks/checks.ts`). The pane, strip and view state are three session state
  values: `antiburn-live.view`, `antiburn-live.strip` and `antiburn-live.pane`.
  For K1, the mod keeps the first 2,000 characters of your latest request in
  memory. For C1, it keeps the approved plan and the requests after it.
- **What it reads.** Native account rate limits in both modes. In checks mode:
  tool call inputs and the size of each result (not its
  text), the model, the effort, token usage, context readings, your prompts,
  the permission mode, and the approved plan. It reads the installed commands
  only after a main turn of 10 or more steps that runs no skill.
- **When it writes state.** At the end of a main turn when a check changed,
  at a prompt or a context reading that changes a check, after a compaction,
  and after a press. The per-step hook makes no engine call and writes no
  state. The `LiveChecks` methods are synchronous and do no I/O.
- **Bounds.** The mod keeps at most 1,000 read targets per turn and 200
  command sets per session. A call key keeps the first 200 characters of a
  long input value.

## Development

From the repository root:

```bash
node scripts/assemble-claude-mod.mjs --dev-test
claude plugin test .agent-artifacts/claude-mod/dev-test/plugins/antiburn-dev
claude plugin validate .agent-artifacts/claude-mod/dev-test/plugins/antiburn-dev
```

The test stage excludes release-source tests and runs development tests against
the generated development identity. Do not run `claude plugin test` directly
on this source folder: the host recursively discovers the separate release
entry's tests, which belong to the public artifact's test stage.

For a source type check, load `plugins/antiburn-live` directly in a separate
interactive test session to generate its local `.claude-plugin/types`, then run:

```bash
npx -y -p typescript@5.6.3 tsc -p plugins/antiburn-live --noEmit
```

Generated declarations reflect the installed host and can include account MCP
types. Keep them ignored and out of both artifacts. CI validates capabilities
and tests both emitted identities; the separate SDK type check remains a local
release gate.

The `policy` part of `hooks/catalog.ts` must agree with the engine catalog
(`ReportCatalogs`): the revisions, the depth cap (D), the effort cap (T), the
premium models (S) and the model replacements (O). A test compares each one,
and it fails when `policy` holds a field that has no test. The `live`
thresholds and the `checks` text exist only in the mod, because the engine has
no copy of them. From the repository root, run:

```bash
cargo nextest run --manifest-path crates/antiburn-local/Cargo.toml --test live_catalog_drift
```

The CI job `antiburn-live mod` validates the marketplace and the mod, refuses
network, process, model and unbounded timer calls, and runs the tests on Claude Code
2.1.292.

### File layout

| Path                          | Contents                                                                                                                                                            |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.claude-plugin/plugin.json`  | The manifest: name, version, description, and the state types.                                                                                                      |
| `hooks/hooks.json`            | Loads `register.tsx`.                                                                                                                                               |
| `hooks/register.tsx`          | The hooks, the `/antiburn-dev` command, the strip, the chip, the pane, the fix actions and the demo.                                                                |
| `hooks/checks.ts`             | `LiveChecks`, the check rules and stages. It takes no `$` and does no I/O.                                                                                          |
| `hooks/catalog.ts`            | The check catalog: ids, names, sentences, fixes, thresholds and model replacements. Keep the text between the `catalog:begin` and `catalog:end` markers valid JSON. |
| `hooks/tour.ts`               | The 4 demo tour steps.                                                                                                                                              |
| `hooks/*.test.ts`             | Tests for the checks, the surfaces and the tour.                                                                                                                    |
| `hooks/usage.ts`              | Pure usage normalization, pace calculations and read ordering.                                                                                                      |
| `hooks/usage-graphics.ts`     | SVG typography, meter drawing and bounded CSS motion.                                                                                                               |
| `hooks/usage-presentation.ts` | Shared geometry, timeline calculations and entrance ownership.                                                                                                      |
| `hooks/usage-font.ts`         | Bundled Bitcount glyph layout; license in `assets/bitcount`.                                                                                                        |
| `types/index.d.ts`            | The check view and the state types.                                                                                                                                 |
| `types/claude-code.d.ts`      | Names the `claude-code` module for tools that read only the committed source.                                                                                       |
| `tsconfig.json`               | Extends the generated `.claude-plugin/types/tsconfig.json`.                                                                                                         |
| `.gitignore`                  | Ignores `.claude-plugin/types/`.                                                                                                                                    |

## Known interaction issues

### The first click in the pane only moves the keyboard

Seen in Claude Code 2.1.296 in the desktop Code tab.

When the prompt holds the keyboard, a click on a pane button moves the
keyboard to the pane but does not press the button. You must click again.
While the pane holds the keyboard, later clicks press at once.

The mod cannot work around this. The host does not raise `ui.press` for that
click, and the `ui.focus` event that it raises has no `element`, so the mod
cannot tell which button you clicked. The mod handles a press that it
receives in a few milliseconds, so the delay is the wait for the second click.

A fix needs a change in Claude Code: the click should press the button, or
`ui.focus` should name the clicked element.

### Developer usage previews

Open `/antiburn-dev debug` to show the separate developer debug pane. The `debug`
argument is unadvertised: it is omitted from the normal command hint and help
response. The host can hide whole commands but has no per-argument hiding setting.
No environment variable or restart is required. Usage defaults to **Label reveal**:
label, adaptive dot track, and percentage share a row per window. Narrow views
stack complete rows. The Usage card's layout buttons select **Current** (captions
above the tracks) for comparison with the same live or synthetic readings.
Desktop dots are 7px wide with a fixed 4px gap (11px center to center) and 85% fill opacity in both layouts; available width changes the count, never the spacing. Layout changes cancel pending motion and reset on plugin reload. Hover a label
for reset text over its track; focus or tap reveals it immediately. Pace has its
own control. Terminals retain the static glyph fallback with label and pace
controls, without graphical animation. The debug pane alone shows the
effective mode: **Mode 0 · Usage limits** or **Mode 1 · Usage limits + live checks**.
The mode button switches between usage-only and live checks for the current
session. Checks start fresh after each change; switching to Mode 0 closes the
checks pane and clears its status. **View live checks** opens the existing checks
pane in Mode 1. Reload restores `ANTIBURN_LIVE_MODE` (`usage` or unset for Mode 0,
`checks` for Mode 1); the button does not change that environment setting. Normal checks panes omit developer controls and mode labels. The separate usage
pane is disabled; current readings and Refresh are in the debug Usage card. Select inactive, low, mid, beyond-pace, high, near-limit,
missing, or read-error examples. Extra 9%, 10%, 99%, and 100% examples
exercise digit rolls. Exceeded and fractional examples are not in the menu. Synthetic data is labeled
in the Usage card and above the meters. Native snapshots continue updating separately;
**Return to live data** restores the latest reading.

The Usage card also offers **Idle**, **Checking**, **Retry**, **Success**, and
**Stale** previews for the account-usage states. These selectors change only the
synthetic preview; they do not read native usage, authorize, request account
data, or submit a model prompt. **Refresh** moves Idle or Retry to Checking in
the preview. The usual Return to live control restores the latest native reading.

The animation inspector lists timings and easing and replays entrance or reset
reveal/hide. Example fill transitions run for 360ms, including decreases and
rapid retargeting. Beyond-pace and high previews color only the used portion to the right of a
reached pace marker red. Dots before it stay orange. Near-limit makes all used
dots red and adds a
subtle glow and six 1600ms opacity pulses, then settles. Reduced motion settles
fill, digits, and warning pulses. Terminal examples retain matching values and
static red glyphs; graphical replay requires desktop SVG support. These warning
styles are synthetic demonstrations, not a production warning policy. Reload or
clear may discard preview selection with host state; native limits never change.

Desktop meters draw their data in SVG images and place reset/pace interactions
in a separate transparent frame. Updating values therefore does not reload the
whole visible meter frame. Identical readings that do not change reset or pace
presentation retain their existing graphic source.

The Rooted flame signals recent generated-output activity. It does not report
subscription quota use or exact instantaneous token throughput. See the
[cross-surface burn activity heuristic](burn-activity.md) for its inputs,
thresholds, expiry, and limitations. The debug card offers separate 15-second
active and reduced-motion previews. Desktop uses a finite Ember catch animation;
terminal layout and rendering are documented in the
[terminal brand decision](terminal-renderers.md).
The desktop logo and terminal wordmark link to the antiburn website, including
during activity. The logo has no activity tooltip; terminal wordmark hover shows
a dim modifier-click hint.

The debug pane's **Play all usage examples** button runs once through entrance,
usage fixtures except numeric-roll examples, reset and pace reveal/hide, then Ember catch on the high
usage fixture (88% / 82%). Each example holds for 1.8 seconds; near limit holds
for 10 seconds to show all six pulse cycles. The Ember finale lasts 4 seconds.
Only this explicit sequence schedules one pending `clock.after` at a time.
Stop, a manual fixture, returning to live data, layout changes, restart, and
session changes cancel the sequence. Terminal shows the same sequence of static
states. Native readings continue in the background. The last example remains
selected when playback finishes; **Return to live data** restores native values.

Synthetic usage examples share fixed pace markers (5-hour 55%, Weekly 50%). Only usage changes between examples; native live pace remains time-based. Inactive/missing windows still omit markers when no reset time exists.

## Terminal brand mark

The terminal uses a monospace `antiburn` wordmark in bright antiburn orange on a
muted orange background beside its meters. Both inline and stacked layouts use
exactly two terminal rows. The lower row carries the bright wordmark and inline
meters; stacked limit rows use the same two-row space. The upper row is blank
padding while idle and carries the shaded-block flames upward when active. The
wordmark slot is 11 columns: a one-cell inset plus ten columns for the eight
linked letters and one background-padding cell on each side. The upper flame
row uses a raster, while linked text retains the heat colors below. The meter gap adds
one more column outside the slot. Image rendering and its experimental debug
controls are removed. See the
[terminal rendering decision](terminal-renderers.md).

Desktop usage percentages show whole numbers, rounded down. The source reading
remains exact; readings below 100% never display as exhausted. At 100% and above,
filled dots and the pace marker use steady red with the existing softened fill.
Pace markers include a 1px right-edge shadow at 22% opacity for dot separation.

# antiburn

antiburn (Mod) shows your account usage limits above the
prompt. It displays the five-hour and weekly windows, reset times, and pace
when Claude Code supplies enough information. Values come from the host's
account usage API. They are not a token counter, a bill estimate, or a promise
of provider freshness.

Version 0.2.0 is the unified Claude Code testing candidate. It
preserves the 0.1.4 usage implementation within the single **antiburn** plugin.
Public output excludes antiburn CLI discovery/access, live checks and debug
previews. Those remain available only in the separate local `antiburn-dev` build.
The desktop app and CLI are not required. Other agent adapters remain unpublished.

## Try the candidate locally

From a checkout of the repository:

```sh
node scripts/assemble-claude-mod.mjs
node scripts/assemble-claude-mod.mjs --check
claude --plugin-dir ./plugins/antiburn
```

Use a separate test session. `/antiburn` shows readable usage details.
`/antiburn refresh` updates usage and returns a short confirmation; failures
explain availability and retain last known readings. `/antiburn help` explains
these two actions. Technical diagnostics and presentation flags belong to
`/antiburn-dev`. Command replies omit repeated branding and the leading plugin label.
Native commands do not call a model and require the enabled Claude mod runtime.
The plugin replaces the standalone antiburn findings skill and owns `/antiburn`.
Remove the old owned skill from the selected scope before installing; see the
[retirement instructions](../../docs/antiburn-shell/migration.md#retire-the-owned-standalone-skill).
There are no alternate command names or legacy aliases.

The manifest option `usageUiEnabled` defaults to true. Disabling it and reloading
stops automatic usage reads, the strip and its display timers. Explicit usage
commands remain available and can request account data; this setting is not a
network-off switch. A supported, enabled mod runtime is required for native
commands and the strip. When mods are disabled or blocked, use this guide; there
is no separate model-mediated help skill.

The usage strip reads native host measurements first, including partial readings.
After Desktop attaches, an empty or unavailable native reading can automatically
fall back to an authenticated account-usage request through the host. The same
path serves Desktop reconciliation, **Refresh**, and `/antiburn refresh`.
Terminal-only sessions continue to use native measurements.

The fallback uses the session's opaque bearer authorization handle for a GET to
`https://api.anthropic.com/api/oauth/usage`, with the
`anthropic-beta: oauth-2025-04-20` header. The credential stays with Claude Code.
API-key sessions and hosts without supported authorization stay unavailable. The
endpoint and beta header are compatibility dependencies; support can vary by
host version and login type. The mod reads no CLI cache, credentials file,
Keychain item, or transcript, and needs no terminal session or antiburn app.

A successful account reading stays fresh for 60 seconds, including explicit
Refresh. Concurrent refreshes share one request. Failures retain previous
readings with an unavailable status and defer new event-triggered attempts for
60 seconds, doubling up to 15 minutes. A longer valid `Retry-After` on HTTP 429
extends that delay. The host enforces a
[30-second HTTP deadline](https://code.claude.com/docs/en/plugins/mods/reference#limits),
including redirects and the response body. There are no usage-fetch or retry
timers, polling, automatic retries, prompt submissions, or model calls. The
terminal Checking indicator animates only while mounted with a lifecycle-owned
300 ms timer. Desktop dots use CSS animation and honor reduced motion; the
terminal SDK exposes no reduced-motion setting. Native measurements always take
precedence; account data never fills missing windows in a valid native reading.

While a read is running and no numeric usage is available, the strip shows only
**Checking usage...**. Refresh returns when the check finishes; repeated clicks
share one check and there is no artificial delay. Once the read settles without
a usable reading, it shows **Usage unavailable** with a dimmed **↻** icon
button where space permits. Existing numeric readings remain visible during
refresh; failures retain those readings with a stale status.

After an account failure, the command says **Account retry at [time] · Refresh
checks session.** The strip shows the same compact line, wrapping at narrow
widths, when it has no numeric reading; retained meters keep their existing
display. Times use the runtime's default timezone with an explicit abbreviation;
UTC is the fallback if formatting is unavailable. No user's timezone is hardcoded.
Seconds and the date appear when needed, rounded upward so the displayed time is
not earlier than the retry boundary. This is not a live countdown. Refresh still
checks native usage while account requests are deferred.

Desktop retry and explanatory text use the existing dim Bitcount glyphs, with
bounded wrapping or truncation and full accessible descriptions. The SDK's native
Refresh button uses its native font with dim styling. This host renders the
button label instead of its child text and offers no separate accessible-name
property, so the icon is also its accessible name; nearby retry copy explains
Refresh. Terminal secondary text
uses the terminal's font and dim styling; the terminal SDK does not offer SVG.

`/antiburn` shows details.
When no reading exists, the command shows “Checking usage…” while the current
request runs. Otherwise it reports that usage is unavailable and gives the
refresh command to try again. Refresh does not send a message to Claude. The secondary
Refresh control is dimmed. Desktop draws it as a native button in its own font;
the status message retains the dot lettering.

Desktop primary captions use 14px Bitcount lettering, with dim 12px secondary
copy and a 13px brand mark. Meter dots are 7px wide with 4px gaps and 85% fill opacity. Desktop bars stay
side by side at content widths of 480px and above; narrower strips use a 1px row
gap. Checking and unavailable states use their single-line content height. Pace markers
use a continuous line, neutral before the threshold and orange once reached.
Terminal typography, dot glyphs, and pace markers retain their existing styling.

The usage strip refreshes on native host measurements. Activate a quota control
to inspect its reset and pace details. The logo opens https://antiburn.com.
Terminal pointer reveals use the host fullscreen renderer (the `tui` setting);
classic terminal rendering retains command and keyboard activation fallbacks.
For keyboard access in the terminal, press Ctrl+X, then Tab to give the usage
strip focus. Then use Tab to move between controls and Enter to activate one.
Plain Tab from the prompt does not enter the strip. Run `/antiburn` for
textual quota, reset, and pace details. Missing values stay unavailable; a failed
refresh can retain the last reading with a stale status. A fresh session starts
with native readings instead of restoring development previews.

This mod targets Claude Code's terminal UI and the Desktop Code tab. It is not
an integration for ordinary Claude Desktop chat or Cowork. The original candidate used Claude Code 2.1.302; unified acceptance targets the
current installed host, with exact results recorded separately.

Earlier 0.1.2 Desktop checks, before the loading-copy correction, covered a fresh empty session: the strip changed from
pending to account meters before any model response. Built-in `/usage` agreed
and reported $0.00 session cost; `/antiburn refresh` retained the readings.
Narrow layouts, pending and error states, the error Refresh action, and logo
website activation were checked. Clearing Desktop removed all mod surfaces
until a local command started the session again. Running `/antiburn-mod` then
restored the pending strip followed by automatic account meters, without a
model response. Clearing does not guarantee that the host keeps the strip mounted.

Earlier 0.1.2 terminal checks, before the loading-copy correction, used Claude Code's actual fullscreen renderer in a PTY.
The native meters and explicit refresh worked with a subscription login;
`/usage` reported $0.0000, zero API duration, and zero input and output tokens.
Pointer events revealed reset and pace details, leaving the control restored the
dots, and logo activation opened the website. At 40 columns, pending guidance
wrapped onto two lines, meters stacked, reset details stayed bounded, and
command details wrapped. These checks did not use an iTerm2 window or a physical
side-by-side screenshot. They establish behavior in the tested PTY, not general
terminal-emulator compatibility. Full accessibility remains unverified.

Earlier candidate checks used Claude Code 2.1.301 for automated validation and
covered Desktop with its terminal pane side by side, reload, terminal resume,
keyboard reset activation, and fullscreen hover reveals. Their clear behavior
was session-specific; the current Desktop behavior is described above. Earlier
sessions sometimes supplied only weekly native usage in Desktop while terminal
supplied both windows. Missing values remain unknown.

An earlier Desktop load warning came from an unrelated MPX mod whose required
plugin was disabled in the synthetic test workspace. Disabling that mod locally
cleared the warning. Earlier release/dev co-load and isolated install checks
covered distinct commands, one shared usage strip, separate install records,
and independent lifecycles. Earlier local git-source update and explicit
uninstall/reinstall rollback rehearsals passed. These historical checks do not
prove hosted transport, session-state migration, or every 0.1.2 lifecycle path.
No Windows, iTerm2, or other terminal compatibility claim is made.

## Distribution status

The `feat/antiburn-mod-release` branch provides the testing marketplace. It
contains only `antiburn`, pinned to an immutable commit containing the generated
`plugins/antiburn` artifact. This is branch-based testing, without a GitHub
release or tag. The main branch catalog may still describe the historical plugin.

```sh
claude plugin marketplace add https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release
claude plugin install antiburn@antiburn --scope user
```

If the `antiburn` marketplace already follows this testing branch, refresh it
before installing with `claude plugin marketplace update antiburn`. Retire the
owned Claude skill in the selected scope first. Check the installed identity,
version and source commit; reload the session after installation.

Do not reuse a released version for different content. Follow the
[plugin page](https://github.com/antiburn/antiburn/blob/main/plugins/README.md)
and the repository's `docs/antiburn-shell/migration.md` for initial launch and local lifecycle checks.
The historical native observations below belong to their stated original
versions; they do not certify every unified candidate path.

## Local development variant

The development implementation remains in `plugins/antiburn-live`. Generate its
isolated local copy with:

```sh
node scripts/assemble-claude-mod.mjs --dev
claude --plugin-dir ./.agent-artifacts/claude-mod/dev/plugins/antiburn-dev
```

The generated local marketplace is `.agent-artifacts/claude-mod/dev`, named
`antiburn-local-dev`. It contains only `antiburn-dev`, with the command
`/antiburn-dev`. Its state owners and command registrations are
rewritten during assembly. The artifact and catalog are ignored local files;
they must not be submitted or published. Development checks are not part of
this public plugin. Burn animation and its previews are temporarily disabled in
the development build; checks, actions and usage controls remain available.

Use separate test sessions for ordinary development. Release/dev co-loading must
preserve the development check pane and notifications while avoiding duplicate
quota reads and meters. Follow the generated dev options and the native evidence
ledger; a historical single-strip observation does not certify composition.
Disable the development variant in release acceptance sessions.

## Rebuild and verify

The public folder is generated from an explicit file allowlist. Edit the
release entry and metadata under `plugins/antiburn-live/release`, or the shared
usage helpers under `plugins/antiburn-live/hooks`, then regenerate it. Do not
edit the generated public copy directly. The assembler copies the root MIT
license and the unchanged desktop icon master. The icon is owner-provided
artwork dated 2026-08-13; `apps/desktop/src-tauri/icons/icon.png` is authoritative.
The Bitcount glyph derivative retains its [SIL OFL notice](assets/bitcount/OFL.txt).

```sh
node --test scripts/assemble-claude-mod.test.mjs
node scripts/assemble-claude-mod.mjs --check
claude plugin validate --strict --json .claude-plugin/marketplace.json > /tmp/antiburn-marketplace-validation.json
node scripts/assemble-claude-mod.mjs --validate-marketplace-log /tmp/antiburn-marketplace-validation.json
claude plugin validate --strict --json plugins/antiburn > /tmp/antiburn-mod-validation.json
node scripts/assemble-claude-mod.mjs --validate-log /tmp/antiburn-mod-validation.json
node scripts/assemble-claude-mod.mjs --test
claude plugin test .agent-artifacts/claude-mod/test
node scripts/assemble-claude-mod.mjs --dev-test
claude plugin test .agent-artifacts/claude-mod/dev-test/plugins/antiburn-dev
node scripts/assemble-claude-mod.mjs --dev
node scripts/check-claude-mod-lifecycle.mjs
```

The plugin and marketplace must pass strict validation with no warnings.
The validation policy rejects every warning and error; no naming exception is
permitted. Directory name availability remains a separate external check.

The test staging command checks the exact public bytes first, then adds test
files only to an ignored copy. The distributed folder contains no test fixtures
or generated host/account declarations. Host SDK type-checking needs declarations
from an isolated session on the pinned host; generated declarations must remain
local. CI validation and runtime tests do not establish native UI support or
replace that type check.

Read the [privacy description](PRIVACY.md) and [changelog](CHANGELOG.md).
Report reproducible problems through [GitHub issues](https://github.com/antiburn/antiburn/issues).
Source code is covered by the included [MIT license](LICENSE).

Desktop usage percentages show whole numbers, rounded down. The source reading
remains exact; readings below 100% never display as exhausted. At 100% and above,
filled dots and the pace marker use steady red with the existing softened fill.
Pace markers include a 1px right-edge shadow at 22% opacity for dot separation.

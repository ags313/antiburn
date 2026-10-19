# antiburn (Mod)

antiburn (Mod) shows your account usage limits above the
prompt. It displays the five-hour and weekly windows, reset times, and pace
when Claude Code supplies enough information. Values come from the host's
account usage API. They are not a token counter, a bill estimate, or a promise
of provider freshness.

Version 0.1.3 is a usage-only testing candidate. It includes an explicit
host-mediated account usage fallback, but no background polling or prompt
actions. Development-only debug previews remain in the local `antiburn-mod-dev` build.
The cross-vendor antiburn core is a separate product and is not required.

## Try the candidate locally

From a checkout of the repository:

```sh
node scripts/assemble-claude-mod.mjs
node scripts/assemble-claude-mod.mjs --check
claude --plugin-dir ./plugins/antiburn-mod
```

Use a separate test session. The command is `/antiburn-mod`; if another
plugin already owns that spelling, use the name Claude Code assigns during
registration. The mod leaves `/antiburn` available for the core plugin.

The usage strip reads native host measurements first, including partial readings.
After Desktop attaches, an empty or unavailable native reading can automatically
fall back to an authenticated account-usage request through the host. The same
path serves Desktop reconciliation, **Refresh**, and `/antiburn-mod refresh`.
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

The default `/antiburn-mod` command shows details; `details` remains accepted.
When no reading exists, the command shows “Checking usage…” while the current
request runs. Otherwise it reports that usage is unavailable and gives the
refresh command to try again. Refresh does not send a message to Claude. The secondary
Refresh control is dimmed. Desktop draws it as a native button in its own font;
the status message retains the dot lettering.

The usage strip refreshes on native host measurements. Activate a quota control
to inspect its reset and pace details. The logo opens https://antiburn.com.
Terminal pointer reveals use the host fullscreen renderer (the `tui` setting);
classic terminal rendering retains command and keyboard activation fallbacks.
For keyboard access in the terminal, press Ctrl+X, then Tab to give the usage
strip focus. Then use Tab to move between controls and Enter to activate one.
Plain Tab from the prompt does not enter the strip. Run `/antiburn-mod` for
textual quota, reset, and pace details. Missing values stay unavailable; a failed
refresh can retain the last reading with a stale status. A fresh session starts
with native readings instead of restoring development previews.

This mod targets Claude Code's terminal UI and the Desktop Code tab. It is not
an integration for ordinary Claude Desktop chat or Cowork. Current automated
and native validation uses Claude Code 2.1.302.

Earlier 0.1.2 Desktop checks, before the loading-copy correction, covered a fresh empty session: the strip changed from
pending to account meters before any model response. Built-in `/usage` agreed
and reported $0.00 session cost; `/antiburn-mod refresh` retained the readings.
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

The preview branch has a GitHub-hosted catalog. Install without a manual clone:

```sh
claude plugin marketplace add "https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release" --scope user
claude plugin install antiburn-mod@antiburn --scope user
```

The testing branch catalog pins an immutable candidate. This 0.1.3 candidate adds
spacing above the strip and stable meter graphics during unrelated redraws.
No Anthropic Directory listing or GitHub release has been created. Restart Claude Code or reload plugins after
installation. In Desktop, use the Code tab and enable the plugin for your project.
See the [maintainer guide](https://github.com/antiburn/antiburn/blob/feat/antiburn-mod-release/plugins/README.md)
for current candidate status, installation procedures, and host limitations.

Do not reuse a released version for different content. Recovery after a public
release requires a newly versioned, reviewed artifact, even when its code
restores earlier behavior.

## Local development variant

The development implementation remains in `plugins/antiburn-live`. Generate its
isolated local copy with:

```sh
node scripts/assemble-claude-mod.mjs --dev
claude --plugin-dir ./.agent-artifacts/claude-mod/dev/plugins/antiburn-mod-dev
```

The generated local marketplace is `.agent-artifacts/claude-mod/dev`, named
`antiburn-local-dev`. It contains only `antiburn-mod-dev`, with the command
`/antiburn-mod-dev`. Its state owners and command registrations are
rewritten during assembly. The artifact and catalog are ignored local files;
they must not be submitted or published. Development checks are not part of
this public plugin.

Use separate test sessions for ordinary development. Desktop and terminal can load both
variants and their distinct commands together, but show one shared usage strip. Disable the development
variant in release acceptance sessions.

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
claude plugin validate --strict --json plugins/antiburn-mod > /tmp/antiburn-mod-validation.json
node scripts/assemble-claude-mod.mjs --validate-log /tmp/antiburn-mod-validation.json
node scripts/assemble-claude-mod.mjs --test
claude plugin test .agent-artifacts/claude-mod/test
node scripts/assemble-claude-mod.mjs --dev-test
claude plugin test .agent-artifacts/claude-mod/dev-test/plugins/antiburn-mod-dev
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

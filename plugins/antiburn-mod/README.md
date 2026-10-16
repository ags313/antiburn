# antiburn (Mod)

antiburn (Mod) shows your native account usage limits above the
prompt. It displays the five-hour and weekly windows, reset times, and pace
when Claude Code supplies enough information. Values come from the host's
account usage API. They are not a token counter, a bill estimate, or a promise
of provider freshness.

Version 0.1.1 is a usage-only release candidate. It does not include live checks,
debug controls, synthetic previews, background polling, or prompt actions.
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

The usage strip refreshes on native host measurements and re-reads usage after
Desktop attaches. Run `/antiburn-mod refresh` or use **Refresh** when readings
are unavailable. These actions do not send a message or call a model. A host
without quota data remains unavailable until Claude supplies a measurement.
The default `/antiburn-mod` command shows details; `details` remains accepted.

The usage strip refreshes on native host measurements. Activate a quota control
to inspect its reset and pace details. The logo opens https://antiburn.com.
Terminal pointer reveals use the host fullscreen renderer (the `tui` setting);
classic terminal rendering retains command and keyboard activation fallbacks. Missing values stay unavailable; a failed
refresh can retain the last reading with a stale status. A fresh session starts
with native readings instead of restoring development previews.

This mod targets Claude Code's terminal UI and the Desktop Code tab. It is not
an integration for ordinary Claude Desktop chat or Cowork. Automated validation
uses Claude Code 2.1.301. Native checks render the candidate in the Desktop Code
tab and its terminal pane side by side. Both show quota strips, return details,
reload without candidate load errors, and retain usage after clearing. Terminal
resume also restores the strip. An older Desktop test session loaded the candidate
after a plugin reload; its host supplied no readings, and the strip showed usage
not available. Website activation, terminal keyboard reset activation, and
fullscreen terminal hover reveals were checked. Native resizing and full
accessibility remain unverified. Desktop may supply only weekly usage while terminal supplies
both windows; unavailable values remain unknown.

The earlier Desktop load warning came from an unrelated MPX mod whose required
plugin was disabled in the synthetic test workspace. Disabling that mod locally
cleared the warning. Release and development variants load together on Desktop and terminal
with distinct commands; the host displays one shared usage strip. Isolated install
records and independent release/dev lifecycle pass. A local git-source cached
update and explicit uninstall/reinstall rollback rehearsal also pass. These checks
do not prove hosted transport or session-state migration. No Windows, iTerm2,
or other terminal compatibility claim is made.

## Distribution status

The preview branch has a GitHub-hosted catalog. Install without a manual clone:

```sh
claude plugin marketplace add "https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release" --scope user
claude plugin install antiburn-mod@antiburn --scope user
```

The catalog pins an immutable candidate. No Anthropic Directory listing or
GitHub release has been created. Restart Claude Code or reload plugins after
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

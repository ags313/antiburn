# antiburn plugins

`antiburn-mod` is the usage-only Claude Code extension. The testing branch
marketplace serves candidate version 0.1.2. The cross-vendor core reserves the
`antiburn` name. Local development uses `antiburn-mod-dev` and is not in the
public catalog.

## Current distribution status

The catalog on `feat/antiburn-mod-release` pins the 0.1.2 candidate at immutable
commit `6de23173eb1c6f7a51381fa79f69cb4a7a09885a`. This is a public testing
branch, not a GitHub Release or Claude Directory listing. No pull request is
required to test through this branch marketplace.

Existing users can update from a shell:

```sh
claude plugin marketplace update antiburn
claude plugin update antiburn-mod@antiburn --scope user
```

Then start a new Claude Code session or run `/reload-plugins` in the existing
session. Use `--scope project` for a project-scoped installation. Individual
users can enable auto-update for antiburn in the terminal's `/plugin`
Marketplaces tab. Auto-update is controlled by the user or managed settings;
the catalog cannot enable it by default.

The branch catalog uses `git-subdir` to fetch only `plugins/antiburn-mod` at its
recorded immutable commit. The distributed candidate files remain unchanged.

The display names are **antiburn (Mod)** and **antiburn (Mod · Dev)**.
Their identifiers remain `antiburn-mod` and `antiburn-mod-dev`. The previous
0.1.0 candidate used the older display name and did not include usage refresh.

## Install before a Directory release

Use Claude Code with mod support. Current automated candidate checks use version
2.1.302; current native checks cover the Desktop Code tab and the actual terminal
renderer in a PTY. Earlier automated candidate checks used 2.1.301. See the
[candidate verification details](antiburn-mod/README.md) for tested interactions
and the Desktop clear behavior.
Ordinary Desktop chat and Cowork are not supported surfaces.

### Install directly from GitHub

No manual clone is needed. Register the preview branch's hosted catalog and
install the release candidate for your user:

```sh
claude plugin marketplace add "https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release" --scope user
claude plugin install antiburn-mod@antiburn --scope user
claude plugin list
```

Claude manages its own download and cache. Git is still required. The branch
suffix is required while the catalog is outside the default branch. This does
not require an Anthropic Directory listing. See the
[official hosted catalog instructions](https://code.claude.com/docs/en/plugins/host-marketplace#host-your-marketplace).

### Install from a checkout

Alternatively, clone the preview branch into a new directory, then register its
catalog and install the release candidate for your user:

```sh
git clone --single-branch --branch feat/antiburn-mod-release https://github.com/antiburn/antiburn.git antiburn-mod-preview
cd antiburn-mod-preview
claude plugin marketplace add . --scope user
claude plugin install antiburn-mod@antiburn --scope user
claude plugin list
```

If you already have this branch checked out, run the last three commands from
that checkout instead. Keep the checkout available while its local catalog is
registered. Installation fetches the plugin from the catalog's pinned GitHub
commit. It does not install the current authoring files or the development
variant. The local 0.1.2 candidate displays **antiburn (Mod)** and adds automatic Desktop
account fallback; the catalog still installs its recorded earlier version.

Start a new Claude Code terminal session, or reload plugins in an existing
session with `/reload-plugins`. In Desktop, open a Code session and check
Settings → Plugins; enable the plugin for the intended project if it is disabled.
Reload that Code session after changing its plugin state. The mod command is
`/antiburn-mod`. Adding this local catalog does not submit a Directory listing.
See Claude's [catalog installation documentation](https://code.claude.com/docs/en/plugin-marketplaces#add-the-marketplace-and-install-the-plugin).

To remove this user installation while keeping its persistent data:

```sh
claude plugin uninstall antiburn-mod@antiburn --scope user --keep-data
claude plugin marketplace remove antiburn --scope user
```

### Try the latest authoring files in one terminal session

From the preview checkout, use this instead of a persistent installation:

```sh
node scripts/assemble-claude-mod.mjs --check
claude --plugin-dir ./plugins/antiburn-mod
```

This loads the branch's **antiburn (Mod)** copy for that invocation. It does not
register a persistent installation for Desktop. The host can write SDK files
into a loaded folder; use an isolated copy for development acceptance.

## Local acceptance

From the repository root, verify the generated release before loading an
isolated copy in a test session:

```sh
node scripts/assemble-claude-mod.mjs --check
node scripts/assemble-claude-mod.mjs --test
claude plugin test .agent-artifacts/claude-mod/test
```

Native acceptance uses an ignored copy because the host writes SDK declarations
into a loaded plugin folder. The release artifact includes an account-usage
fallback and no developer preview controls. Debug previews are available only
in the local development mod. There is no background usage polling; the pending
indicator uses only a mounted display animation. Desktop and terminal website activation open only the fixed
antiburn website after a user action.

Generate the local development variant separately:

```sh
node scripts/assemble-claude-mod.mjs --dev
claude --plugin-dir .agent-artifacts/claude-mod/dev/plugins/antiburn-mod-dev
```

Release and development variants have separate command and state identities.
Both can load in Desktop and terminal; the host displays one shared usage strip.
Use release alone for ordinary acceptance. See the
[development guide](antiburn-live/README.md) for experimental features.

## Usage before the first message

The 0.1.2 candidate reads native host usage first. After Desktop attaches, an
empty or unavailable native reading can fall back to the account-usage endpoint
through the host's opaque bearer handle. Partial native readings win without
account fill. Terminal-only sessions retain native behavior.

Desktop refresh and lifecycle reconciliation use the same path. Successful
fallbacks stay fresh for 60 seconds, including explicit refresh; failures use
60-second exponential backoff capped at 15 minutes, or a longer valid HTTP 429
Retry-After. Requests are event-triggered and deduplicated. There are no prompt
or model calls, usage-fetch or retry timers, polling, automatic retries, or
credential/cache reads. The terminal pending indicator has a lifecycle-scoped
300 ms display timer only while mounted; Desktop dots use CSS with reduced-motion
handling.

Without numeric usage, the strip shows one **Checking usage...** status while a
read runs. The **↻** button returns afterward; repeated clicks share one check. Settled
empty readings show **Usage unavailable**. Existing meters remain visible during
refresh; failures retain them with a stale status.

Deferred account checks show **Account retry at [time] · Refresh checks session.**
in the command and, without numeric readings, as one wrapping line in the strip. Time formatting uses the runtime's default timezone with
an explicit abbreviation and a UTC fallback. Desktop secondary descriptions use
dim Bitcount glyphs; the native Refresh button and terminal text retain their
SDK-supported fonts with dim styling. The host uses the icon as both the visible
and accessible button label; no separate accessible-name property is available. There are no timers or artificial delays.
See the [candidate privacy description](antiburn-mod/PRIVACY.md) for the data flow.

## Plugin icon limitation

Claude Code does not read the plugin manifest's `icon` field. The generic plug
icon in Claude's installed-plugin list is expected; plugin metadata cannot
replace it through a documented setting. This also applies to local installs.
See the [official manifest reference](https://code.claude.com/docs/en/plugins/manifest-reference#directory-listing-fields).

The release already includes `assets/icon.png`, copied from the antiburn desktop
icon, and its manifest references `./assets/icon.png`. Anthropic uses this field
for the Directory listing. The bundled icon does not establish Directory
acceptance; verify its appearance in the portal before submission.

## Before public listing

Verify a clean install from the hosted pinned catalog before public listing. Verify Directory name, icon, and source validation in the
portal before submitting. The CLI cannot establish Directory acceptance.
Keep the plugin version in its manifest only. Any later distributed change needs
a new version and a newly verified immutable source.

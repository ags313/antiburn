# antiburn plugins

`antiburn-mod` is the usage-only Claude Code extension. The current preview
candidate is version 0.1.1. The cross-vendor core reserves the `antiburn` name.
Local development uses `antiburn-mod-dev` and is not in the public catalog.

## Current distribution status

The candidate is available on GitHub on `feat/antiburn-mod-release`.
No pull request, GitHub release, or Claude Directory submission has been created.
Pull request creation and marketplace submissions remain on hold pending owner approval.

The branch catalog uses `git-subdir` to fetch only `plugins/antiburn-mod` at its
recorded immutable commit. Version 0.1.0 was verified through a fresh isolated
GitHub install matching all 19 distributed files. That transport evidence does
not establish a hosted upgrade to 0.1.1; verify the new pin separately.

The display names are **antiburn (Mod)** and **antiburn (Mod · Dev)**.
Their identifiers remain `antiburn-mod` and `antiburn-mod-dev`. The previous
0.1.0 candidate used the older display name and did not include usage refresh.

## Install before a Directory release

Use Claude Code with mod support. Automated candidate checks use version
2.1.301; native acceptance covers the terminal and the Desktop Code tab.
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
variant. The 0.1.1 candidate displays **antiburn (Mod)** and includes usage refresh.

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
into a loaded plugin folder. The release artifact contains no live-check engine,
debug controls, fixtures, background polling, or model calls. Desktop and terminal
website activation open only the fixed antiburn website after a user action.

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

The 0.1.1 candidate reads native host usage at session start and again after a
Desktop client attaches. When readings are unavailable, the strip shows
**Waiting for usage data** and a **Refresh** action where space permits.
`/antiburn-mod refresh` also reads the host figures again. These reads do not
send a prompt, call a model, or fetch account data directly. There is no polling.

The host may have no quota figures before its first API response. Refresh can
recover figures that arrive after startup, but cannot force Claude to fetch
missing figures. The strip updates when the host supplies a native measurement.
The host must start its mod runtime before any of these controls can appear.

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

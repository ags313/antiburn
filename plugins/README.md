# antiburn plugin

One antiburn plugin per agent. The first unified candidate targets **Claude Code**,
with account usage meters, reset times, pace, explicit help and usage commands.
The desktop app and antiburn CLI are not required.

The unified **0.2.0 testing candidate** is installable from the
`feat/antiburn-mod-release` branch. Its catalog serves only `antiburn`, pinned to
an immutable artifact commit. Source and initial distribution remain in this
repository. This testing branch creates no release or tag; the main branch
catalog can still refer to the historical `antiburn-mod` artifact.

Read the [Claude plugin guide](antiburn/README.md), [privacy description](antiburn/PRIVACY.md)
and [migration guide](../docs/antiburn-shell/migration.md).

| Build             | Identity       | Included                                                                                                 |
| ----------------- | -------------- | -------------------------------------------------------------------------------------------------------- |
| Public candidate  | `antiburn`     | Native usage, optional meters, concise usage/help/refresh commands, authenticated Desktop usage fallback |
| Local development | `antiburn-dev` | Development checks, actions and CLI discovery alongside quota controls                                   |

The public artifact physically excludes live checks, debug previews and antiburn
CLI discovery/access. Turning a setting on cannot enable those omitted modules.
The local development catalog is separate and must not be published.

## Install the testing candidate

```sh
claude plugin marketplace add https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release
claude plugin install antiburn@antiburn --scope user
```

For an existing marketplace following that branch, run
`claude plugin marketplace update antiburn` before installing. Reload the session.
Burn animation and its previews are disabled in the local development
build until refined. Other development checks and actions remain available.

## Try the local candidate

From a source checkout:

```sh
node scripts/assemble-claude-mod.mjs
node scripts/assemble-claude-mod.mjs --check
claude --plugin-dir ./plugins/antiburn
```

Use a disposable Claude profile/project for validation. Start with `/antiburn help`;
`/antiburn` shows usage details and `/antiburn refresh` updates usage with a short
confirmation. These commands do not send a model message. The plugin owns `/antiburn` and
replaces the old standalone findings skill. Follow the
[owned-skill retirement instructions](../docs/antiburn-shell/migration.md#retire-the-owned-standalone-skill)
before installing in that scope. Native `/antiburn help` and the guide are the
help routes; no alternate command or separate help skill is bundled.

A supported mod runtime is required for native commands and meters. Ordinary
Claude Desktop chat and Cowork are outside this integration. The current candidate has fresh Claude Code 2.1.303 Terminal and Desktop Code
checks, including deterministic commands and Desktop usage before inference.
See the [validation record](../docs/antiburn-shell/validation.md) for exact scope
and remaining native certification limits. No broader platform certification
is implied by the reusable adapters.

## Initial launch

There are no real users of the old `antiburn-mod` identity. The 0.1.4 snapshot
remains a regression baseline; production rename, cache migration and independent
legacy recovery machinery are unnecessary. Validate fresh installs and updates
of `antiburn`, independent local development installs, and safe removal. The old owned
standalone skill is replaced by the plugin. See the
[launch and lifecycle guide](../docs/antiburn-shell/migration.md).

## Other agents and a separate repository

Codex, Cursor, OpenCode and Pi adapters remain unpublished candidates. They share
the private [`antiburn-plugin-core`](../packages/antiburn-plugin-core/README.md)
package. Their release, installation and native compatibility checks are deferred;
Claude's quota UI does not imply quota support on those agents.

This folder is the plugin's own GitHub page within the existing repository. A
separate distribution repository can come later: export the same generated
artifacts, retain source provenance, verify upgrades/rollback and leave a forwarding
page here. Repository creation and publication require separate authorization.

Use [GitHub issues](https://github.com/antiburn/antiburn/issues) for support.
Plugin versions are independent of desktop app versions. No Directory submission,
GitHub release, tag, new repository or main-branch catalog switch is part of this change.

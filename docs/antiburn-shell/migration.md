# Initial Claude launch and local lifecycle

The unified candidate is `antiburn` 0.2.0. There are **no real users** of the old
`antiburn-mod` identity, so the initial launch does not need a production identity
migration, append-only rename history or an independent 0.1.4 recovery catalog.
The preserved 0.1.4 source/artifact snapshot is a behavior regression baseline.

## Local validation

Use disposable profiles and projects. Keep original source worktrees and the
user's unrelated installations intact. Validate the public and development
artifacts independently, then together:

1. Fresh install, enable/disable, update, uninstall with retained data, reinstall
   and removal in user, project and local scopes.
2. Exact installed public file bytes, isolated Git cache behavior and a controlled
   current-version update. Local file transport does not prove hosted delivery.
3. Separate public `antiburn` and local `antiburn-dev` records, commands and state.
   Development checks remain accessible when release owns the usage strip.
4. Full quota and development regression suites, including clear/resume, reload,
   pending requests, stale events, check actions and teardown.
5. Native Terminal and Desktop Code observations against exact artifact hashes.
   Synthetic fixtures must remain labeled; historical observations do not certify
   a new artifact. Older hosts and other surfaces need their own evidence.

Run the reusable local rehearsal after generating both builds:

```sh
node scripts/assemble-claude-mod.mjs
node scripts/assemble-claude-mod.mjs --dev
node scripts/check-claude-mod-lifecycle.mjs
```

It creates temporary profiles, projects and signed disposable Git fixtures. It
must not modify production profiles, shared catalogs, authentication files or
unrelated user skills.

## Initial public identity

The first public install uses `antiburn@antiburn`. Prepare the catalog locally
with the stable marketplace name and an immutable source commit for
`plugins/antiburn`. Do not publish local filesystem URLs, placeholder SHAs or
mutable artifact pins. No old-name rename or forced removal is required.

The testing branch provides the initial catalog. Installation uses:

```sh
claude plugin marketplace add https://github.com/antiburn/antiburn.git#feat/antiburn-mod-release
claude plugin install antiburn@antiburn --scope user
```

Select the intended scope and reload both Terminal and Desktop Code sessions.
Confirm `/antiburn help`, `/antiburn` and `/antiburn refresh`. The
plugin registers `/antiburn` directly. It does not probe the command inventory,
register another name, or retain `/antiburn-mod` as a compatibility alias.
Native commands require an enabled mod runtime; README documentation remains
available when mods are blocked.

## Retire the owned standalone skill

The plugin replaces our standalone antiburn findings skill. Findings and CLI
behavior belong to the local `antiburn-dev` profile, not the public Claude
artifact. Retire the old installation in the same user/project scope before
installing the plugin. Inspect the skill source and confirm it is our old
findings skill (the instructions invoke `antiburn checks <session_id>`); the
name alone does not identify a file that may be removed.

| Agent       | Targeted retirement                                                                                                                                                                  | Replacement                                                                            |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| Claude Code | Inspect the selected user/project antiburn skill source and remove that old owned skill directory. Reload the session.                                                               | Native `/antiburn`; local `/antiburn-dev` for findings and CLI.                        |
| Codex       | Inspect the standalone antiburn skill source shown by the skill manager, including the older `~/.codex/skills/antiburn` location when present. Remove only the confirmed owned copy. | The candidate plugin's `antiburn` skill; use the host-discovered qualified invocation. |
| Cursor      | Inspect the old standalone findings skill and any previous owned `antiburn-plugin` candidate in Customize. Remove those owned copies before loading the replacement.                 | The candidate plugin's `/antiburn` skill.                                              |
| OpenCode    | Inspect the owned `commands/antiburn.md` in the selected scope; replace that command asset when installing the candidate. Do not install a second copy in another scope.             | The candidate's canonical `/antiburn` command asset.                                   |
| Pi          | Disable the previous owned antiburn extension entry before enabling its replacement. Preserve unrelated extension settings.                                                          | The candidate's canonical `/antiburn` extension command.                               |

This is a targeted installation step, not an automatic migration engine. The
builder and plugin do not scan, rename or delete user/project skills. Test with
disposable profiles first; other-vendor installation remains deferred. If host
registration fails, quota remains independent and the host debug log reports the
failure. No alternate invocation is created.

Public usage reads and the strip start automatically. Installation requires no
configuration flags, and the retired public `usageUiEnabled` setting is ignored.
The public artifact physically excludes CLI discovery,
live checks and previews regardless of options or environment flags.

## Development build

The separate local catalog is `antiburn-local-dev`, containing `antiburn-dev`.
Do not add it to the public catalog. Old development copies can be removed
explicitly in their own scope before installing the new local artifact; no
production migration automation is required. Keep public and development source
and tests distinct, and preserve the existing checks/debug functionality. Burn
animation and its preview controls stay disabled until refined; checks, actions
and usage controls remain available.

## A later separate repository

Keep source and tests here initially. A later generated distribution repository
can export the same self-contained artifact plus licenses, source commit and
hashes. Preserve the identity and command/state namespaces where possible, verify
fresh install/update/removal and leave a forwarding page here. Repository creation
and publication remain separately authorized actions.

# antiburn plugin architecture

The unified Claude candidate is one **antiburn** plugin with automatic account
usage UI. Its public artifact excludes CLI discovery/access, live checks and
debug previews. Those capabilities remain in the separate local development
build and unpublished adapters. See the [plugin page](../../plugins/README.md)
and [migration guide](migration.md) for current scope and distribution status.

The plugin owns the canonical `antiburn` entry in each adapter and replaces our
standalone findings skill. Claude registers `/antiburn` directly; local development
uses `/antiburn-dev`. There is no command inventory probe, alternate registration,
legacy command alias or separate model-mediated help skill in the public Claude
artifact. The release/dev capability handshake remains to prevent duplicate quota
reads and strips.

The private shared package is `antiburn-plugin-core`. Claude composes its
release-safe metadata with the existing quota implementation. Codex, Cursor,
OpenCode and Pi remain future release candidates; preserve their tests without
claiming Claude quota support for those agents.

The sections below describe development/future-agent CLI discovery. A found
executable is not proof of its publisher, version, or an app connection. Discovery
never launches the candidate. It is physically absent from the public Claude build.

## Session output

The default marker is 🔥, followed by the brand name in code delimiters. A startup notice is one line:

```text
🔥 `antiburn` · CLI found
```

The other states are `CLI not found` after a completed search and `CLI check unavailable` when the host cannot finish the check. Neither blocks the coding session. An explicit status request can repeat the check.

Expanded status always states the connection limit:

```text
🔥 `antiburn`
CLI: Found
App connection: Not checked
```

Markdown surfaces render the brand name as inline code. Plain native APIs preserve literal backticks. ASCII output omits the emoji and retains the code delimiters. Native orange branding is optional; status remains normal foreground text. Words carry meaning without color. No ANSI escapes are emitted into model text. The flame emoji is the default; --ascii omits it and --flame remains a compatibility alias. See the artifact's options for overrides.

A startup event, explicit command, and model-mediated skill have different guarantees. Skills and prompt commands need the host's permitted tools and can consume model capacity; they cannot guarantee automatic startup. Read [installation](installation.md) and [compatibility](compatibility.md) for each route's limits.

## Build and check

From the repository root:

```sh
node scripts/assemble-antiburn-shell.mjs
node --test packages/antiburn-plugin-core/test/*.test.mjs
node scripts/assemble-antiburn-shell.mjs --verify
```

For formatter, linter, and pinned host SDK type checks, follow [development checks](../../packages/antiburn-plugin-core/README.md).

Generated bundles live under ignored `.agent-artifacts/antiburn-shell/`. Each contains its required shared files and license. Catalogs are generated alongside packages instead of changing shared marketplaces. The source package is separate from the desktop workspace; shell assembly does not install dependencies or change the root lockfile.

Use the generated artifact for host validation. A simulated host does not prove native UI, trusted loading, marketplace approval, Windows/Linux behavior, or remote-host support. Record those separately in the compatibility ledger.

## Privacy and boundaries

Discovery inspects only candidate paths and bounded executable search locations. It must never run `antiburn --version` or another guessed flag: today's executable may launch the desktop app. App-bundle presence alone is insufficient. GUI-launched hosts can have a different PATH from a terminal.

No analytics events are added: this shell has no app bridge and transmits no availability, paths, or host information. Future communication needs a separately designed CLI interface and local security boundary. No success message says “Connected” in this milestone.

See [distribution channels](distribution.md) for public, team, private, and local release gates.

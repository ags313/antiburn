# antiburn plugin core

This private Node >=22 package holds shared product metadata, bounded command parsing, help output, and development adapters. `product.mjs` is safe for the unified Claude release. `core.mjs`, `discovery.mjs`, and `probe.mjs` support local shell candidates and stay out of the public artifact.

From the repository root:

```sh
node --test packages/antiburn-plugin-core/test/*.test.mjs
node scripts/assemble-antiburn-shell.mjs
node scripts/assemble-antiburn-shell.mjs --verify
```

The ignored output is `.agent-artifacts/antiburn-shell/`, with one Claude release candidate delegated to `releaseFiles()` and local-only candidates for deferred hosts. The manifest and `SHA256SUMS` record deterministic bytes. Reassembly refuses unowned, changed, or symlinked output. No archive or published package is produced.

Use the package scripts for formatting, lint, and JSDoc type checks. Tool binaries can be supplied through `ANTIBURN_PRETTIER`, `ANTIBURN_ESLINT`, and `ANTIBURN_TSC`; otherwise the corresponding executable must be on PATH. No tool is downloaded by these scripts.

Type checking requires actual declarations, not hand-written SDK substitutes. Set these absolute paths before running `node packages/antiburn-plugin-core/check.mjs typecheck`:

| Variable                     | Declaration used for this candidate                        |
| ---------------------------- | ---------------------------------------------------------- |
| `ANTIBURN_CLAUDE_TYPES`      | Claude Code 2.1.301 generated `claude-code/index.d.ts`     |
| `ANTIBURN_PI_TYPES`          | `@earendil-works/pi-coding-agent` 0.80.3 `dist/index.d.ts` |
| `ANTIBURN_OPENCODE_TYPES`    | `@opencode-ai/plugin` 1.19.2 `dist/index.d.ts`             |
| `ANTIBURN_OPENCODE_SDK_ROOT` | `@opencode-ai/sdk` 1.19.2 package root                     |
| `ANTIBURN_NODE_TYPES`        | Installed `@types/node` package directory                  |

SDK declarations stay outside shipped artifacts. Native Claude tests are included in the pinned SDK type check and also run with `claude plugin test`; the host runbook records their runtime evidence. Node adapter tests exercise the actual source with controlled host interfaces. Those tests establish contract behavior, not host compatibility.

The resolver reads only PATH, PATHEXT, ANTIBURN_EXECUTABLE, and filesystem metadata. An explicit executable override must be absolute. It accepts only executable regular files on POSIX and COM/EXE/BAT/CMD candidates on Windows. Missing PATH, unsupported platform or extensions, inaccessible metadata, and an incomplete bounded search produce `unavailable`. It does not return candidate paths, run candidates, read transcripts, or contact the app.

The native Claude adapter launches Node with a fixed argument vector solely to run this resolver. The candidate antiburn executable is never a process argument. If Node or the host process API is unavailable, the output says the CLI check is unavailable. The Pi and OpenCode adapters call the resolver directly in their host runtime.

Prompt-based adapters embed the canonical instructions from `content.mjs` and use permitted host tools. They may consume model capacity and do not promise byte-exact output or automatic startup. See the maintained host installation and certification records in `docs/antiburn-shell/` before claiming a supported surface.

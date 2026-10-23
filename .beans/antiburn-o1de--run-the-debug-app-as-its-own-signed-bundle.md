---
# antiburn-o1de
title: Run the debug app as its own signed bundle
status: todo
type: task
priority: normal
tags:
    - desktop
    - dev-tooling
    - size-s
created_at: 2026-10-23T20:47:37Z
updated_at: 2026-10-23T20:47:37Z
---

On 2026-10-23 the operator's dev app lost access to the LAN: from 05:23 every SSH call it made to mini395 (`10.0.10.39`) failed with `No route to host`, while the same SSH from a terminal worked. Hypothesis, matched by the timing: macOS Local Network privacy judges a child process by its responsible app. The app runner (`~/.local/share/bob-mac/antiburn/run-app.sh`, outside this repo) had been started from a Claude app instance; when that instance restarted at 05:23:29, the dev app's SSH calls lost local network access. Restarting the runner from the operator's terminal fixed it (sync succeeded at 21:42). The dev app runs as a raw `tauri dev` binary that is ad-hoc signed with an identity that changes on every build (for example `antiburn-58c205f3877c28e0`), so it cannot hold a Local Network permission of its own.

- Add a run mode (for example `pnpm --filter @antiburn/desktop app:debug`, via `scripts/run-tauri.mjs`) that builds the debug bundle (the existing `dev:bundle` build with `tauri.debug.conf.json`, identifier `ai.antiburn.desktop.debug`), signs it, and launches it with `open -W -n <bundle>`. LaunchServices then makes the bundle its own responsible process.
- Signing: when `ANTIBURN_DEV_SIGNING_IDENTITY` names a code-signing identity in the login keychain, sign the bundle with it (`codesign --force --deep --sign`), so the designated requirement stays the same across rebuilds. Without it, sign ad hoc and print that Local Network permission may not survive a rebuild.
- Stay in the foreground until the app quits, and turn INT or TERM into a normal quit of the app, so a supervising runner can stop it and wait for the database to close.
- Document in `apps/desktop/README.md`: creating a local self-signed code-signing certificate once (Keychain Access → Certificate Assistant → Create a Certificate, type Code Signing), granting Local Network on the first launch, and that frontend changes need a rebuild in this mode (no Vite).
- Test in `scripts/run-tauri.test.mjs`: the build, sign, and open commands for both signing cases, and the signal handling, with the process calls faked.
- Check (manual, report the result in the bean): launched from a shell that was started by launchd with no app parent, the app's SSH reaches a LAN host after one Local Network grant, and still does after a rebuild signed with the same identity. Label the result: it confirms or refutes the hypothesis above.
- Follow-up outside this repo: the bob session switches `run-app.sh` to this mode once it lands.
- AC: the debug app runs as its own bundle, keeps its Local Network permission across rebuilds when a stable identity is set, and stops cleanly on TERM.

## Acceptance

- [ ] the AC above holds and the tests that cover it exist and pass
- [ ] `node --test apps/desktop/scripts/run-tauri.test.mjs`, `pnpm --filter @antiburn/desktop lint` and `format` are clean
- [ ] `apps/desktop/README.md` documents the mode and the one-time setup
- [ ] commit with `git commit -s`; the message does not mention Claude and has no Co-authored-by

# Remote sessions

Antiburn can copy supported coding-agent sessions from a remote host over SSH
and analyze the copies on your computer. Manage hosts in **Sessions step settings →
Remote hosts**. Use the **Source** filter in Sessions to combine Local with any
remote hosts. A small computer icon identifies a remote session; its tooltip
names the host.

The helper supports Linux x64 and ARM64 hosts. It syncs these sources:

| Agent | Synced source | Limits |
| --- | --- | --- |
| Claude Code | JSONL transcripts with subagents, sidecars, and fork parents | None beyond the accepted format |
| Codex | Rollout JSONL with linked child rollouts | None beyond the accepted format |
| Cursor | CLI agent transcripts under `~/.cursor/projects/*/agent-transcripts/` | No token usage or cost. Chat stores, IDE composer data, and ACP sessions are not synced. The project folder is empty when local discovery cannot decode it. |
| OpenCode | One session and its child sessions from `opencode.db` | The legacy JSON storage is not synced |
| Antigravity | `conversations/<id>.db` with its brain transcript | Brain-only, workspace, and mirror sources are not synced |
| Junie | `~/.junie/sessions/session-<id>/events.jsonl` | Token usage only. JetBrains AI Assistant task history and Junie logs are not synced |

Other agents and remote operating systems are not supported. The desktop's
generic wording does not imply parity with every locally supported source. See
[session coverage](session-coverage.md) and [check coverage](check-coverage.md)
for the evidence limits.

Cursor, OpenCode, and Antigravity need a helper and a desktop from a release
that supports them. Junie needs a helper and a desktop from a later release. Before each scan, the desktop asks the helper which agents
it supports. An older helper receives the original request and syncs only
Claude Code and Codex. An older desktop never requests the new agents.

## Set up a host

1. Configure an SSH host alias on the computer running Antiburn. Verify that
   `ssh your-alias` reaches the intended host with key authentication. Verify its
   host key through your usual trusted channel. Antiburn requires an existing
   trusted key and never accepts a new or changed host key automatically.
2. Download the helper archive for the host's architecture from an official
   [Antiburn application release](https://github.com/antiburn/antiburn/releases).
   Choose `antiburn-remote-<version>-x86_64-unknown-linux-musl.tar.gz` for x64 or
   `antiburn-remote-<version>-aarch64-unknown-linux-musl.tar.gz` for ARM64. Helper
   archives accompany releases that include remote-session support; older
   releases do not contain them.
3. Download that release's `SHA256SUMS`, and check the archive against its entry
   before extraction. On Linux, `sha256sum --check --ignore-missing SHA256SUMS`
   checks the downloaded files; confirm the helper archive reports `OK`.
4. Extract the archive on the remote host. From its extracted directory, run:

   ```sh
   mkdir -p ~/.local/bin
   install -m 755 antiburn-remote ~/.local/bin/antiburn-remote
   ```

   `antiburn-remote --version` reports the release the helper came from. It
   matches the version in the archive name. Use it to identify a helper that is
   already installed.

5. In Antiburn, choose **Add host**, enter the SSH alias and an optional display
   name, then check the connection. If setup is incomplete, follow the displayed
   guidance and check again. Save after the check succeeds.

The helper is a small command-line executable invoked on demand over SSH. It
does not install a desktop app, run a persistent service, or open a listening
port. Antiburn does not install it or execute installation commands for you.
SSH configuration and private keys stay under your control. Up to eight hosts
can be configured.

## Sync and offline access

Automatic sync defaults to every five minutes while Antiburn is open. The next
interval starts after the previous scan finishes, including a failed scan.
Changing the interval during a scan applies after that scan. **Manual only**
disables automatic scans; **Sync now** remains available for enabled hosts.

Each host has a sync switch. Turning it off disables both scheduled and manual
syncing, shows **Sync off**, and disables **Sync now** and **Retry**. Saved
sessions remain available. Pending scans are discarded; an active SSH request
may settle, but the canceled scan cannot commit further updates. Turn the host
on again to resume syncing.

Each scan returns at most 200 supported sessions from the last seven days,
ordered newest first within bounded discovery. Directory-entry, candidate,
preview-byte, and elapsed-time budgets also bound discovery work, so a crowded
store can return a truncated result. A session missing from this bounded list
is not treated as deleted.
Previously synced sessions remain readable offline under the desktop's retention
policy. The host row counts all saved sessions, including sessions outside the
discovery window; opening that list keeps the Sessions date filter. A rejected
session does not prevent other sessions in the scan from syncing. The row shows
the number rejected and keeps the previous fully successful sync time. Saved
counts update after imports, deletion, and retention. SSH, cancellation, and
local storage failures stop the scan. Older helpers that cannot distinguish a
session rejection also stop the scan on failure.
**Retry** requests another scan; a failed SSH connection does not
establish that the host is offline.

The private cache contains transcripts and the supported companion files needed
for analysis. A bundle is limited to 512 files and 1 GiB. The cache is limited to
14 GiB, with a separate 2 GiB staging allowance. A failed or interrupted transfer
preserves the previous good copy. Very large sessions may exceed these bounds.

Changing a display name does not require a connection. Changing an SSH alias
requires a successful connection check and retains the host's identity. Removing
a host deletes its synced copies and analysis from this computer. It does not
delete or modify the originals on the remote host.

## What a synced session can show

Cursor, OpenCode, Antigravity, and Junie sessions use the same discovery as a
local scan on the host. Before an export reads a source, the helper admits the source
file beneath its store root. Symlink components and non-regular files are
rejected. A Cursor export must match the admitted transcript bytes. The helper
opens a database read-only by its path, because SQLite finds the write-ahead log
from the path. It then checks that the path still names the admitted file.

The helper never sends a live database file. It reads one consistent snapshot in
a read transaction and sends the snapshot. An Antigravity export sends the whole
conversation database. An OpenCode export sends a new database that holds only
the `session`, `message`, and `part` rows of the session and its child sessions,
with the source table definitions. Other OpenCode tables stay on the host. Each
scan builds the snapshot again to compare it with the cached copy, and an
unchanged session sends no data. A Junie export sends the session's
`events.jsonl` as it is.

Analysis uses the accepted copied evidence. It does not infer live activity or
borrow this computer's agent configuration, provider account, or quota.
Per-session findings remain subject to the copied evidence's own completeness
and source contract.

Overview's cost totals and daily spend chart include synced remote sessions,
priced with this computer's pricing snapshot. Each session counts on the day of
its last copied activity. Remote sessions have no provider account binding, so
they are excluded from account quota. They are also excluded from Overview's
Subscription view and Recent sessions list, the popover's usage figures, global
Burn Checks, and live HUD calculations.

Local folder/transcript actions, Auto Fix, and remediation verification are
unavailable for remote sessions. Remote copies never authorize editing the
original host. No transcript content, host name, SSH alias, path, or arbitrary
connection error is sent to analytics.

The helper opens transcripts and companion files beneath the supported agent
stores using directory-relative descriptors. It rejects symlink components and
non-regular files, and parses or transfers from the admitted descriptors.
Title and child-label reads have separate byte budgets and use a neutral
fallback when a title is outside the bounded preview. This boundary does not
prevent the host account owner from editing an allowed file or placing a hard
link in the store.

## Build from source

The helper has a standalone Cargo workspace:

```sh
cargo build --manifest-path crates/antiburn-remote/Cargo.toml --release --locked
```

Build on the target Linux architecture. A build from source reports the version
in `crates/antiburn-remote/Cargo.toml`, which a release holds equal to the
application version. The release workflow produces static
musl binaries for both architectures, tests the native helper, rejects binaries
that require a dynamic loader or shared libraries, names each archive from that
same manifest, and includes the archives
in the application release's `SHA256SUMS`. Public-repository releases also include
build provenance; private-repository releases skip attestation. Running local builds
or tests does not create or publish a release.

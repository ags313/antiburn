# Remote sessions

Antiburn can copy supported coding-agent sessions from a remote host over SSH
and analyze the copies on your computer. Manage hosts in **Sessions step settings →
Remote hosts**. Use the **Source** filter in Sessions to combine Local with any
remote hosts. A small computer icon identifies a remote session; its tooltip
names the host.

The helper supports Linux x64 and ARM64 hosts, and Windows x64 hosts that run
the OpenSSH server. It syncs these sources:

| Agent | Synced source | Limits |
| --- | --- | --- |
| Claude Code | JSONL transcripts with subagents, sidecars, and fork parents | None beyond the accepted format |
| Codex | Rollout JSONL with linked child rollouts | None beyond the accepted format |
| Cursor | CLI agent transcripts under `~/.cursor/projects/*/agent-transcripts/` | Synced transcripts carry no token usage or cost. With **Read Cursor account usage** enabled, Overview and session attribution can still show tokens and recorded cost from this Mac's Cursor IDE login when `conversationId` matches. Chat stores, IDE composer data, and ACP sessions are not synced. The project folder is empty when local discovery cannot decode it. |
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

## Set up a Windows host

Windows support is new and has no release archive yet. Build the helper from
source for `x86_64-pc-windows-gnu` (see [Build from source](#build-from-source)).

1. In an administrator PowerShell on the Windows host, install and start the
   OpenSSH server:

   ```powershell
   Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
   Start-Service sshd
   Set-Service -Name sshd -StartupType Automatic
   ```

   Keep `cmd.exe` as the OpenSSH default shell. Antiburn finds a Windows host
   on first contact from the English-only message that `cmd.exe` prints for
   the Linux helper path, then runs
   `"%USERPROFILE%\.local\bin\antiburn-remote.exe" stdio`. The quotes allow
   spaces in the user profile path. First contact with a non-English `cmd.exe`
   is not detected. Antiburn saves the platform reported by `Hello` on the host
   record; later requests and exports use that platform directly, including
   after an app restart. Older host records detect and save it on their next
   successful contact. Changing an alias checks and saves the new host's platform.
2. Add the public key of the computer running Antiburn. For an account in the
   Administrators group, OpenSSH reads
   `C:\ProgramData\ssh\administrators_authorized_keys`, not the key file in
   the user profile.
3. Copy `antiburn-remote.exe` to `%USERPROFILE%\.local\bin\`.
4. Add an SSH host alias as for a Linux host. When the computer also starts
   Linux at the same address, give the Windows alias its own `HostKeyAlias`, so
   the two host keys do not replace each other.

A Windows host can sync the same agents as a Linux host. OpenCode on Windows
keeps its database in `~\.local\share\opencode\` or `AppData\Roaming\opencode\`,
and local discovery checks both.

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

Each host has a **Sync history** choice: the last 7 days, 30 days, 90 days, 6
months, or year, or all history. Seven days is the default. A 7-day scan returns
at most 200 supported sessions, and a longer one returns at most 1,000, ordered
newest first within bounded discovery. A longer history needs a helper from a
release that supports it. An older helper rejects the longer window, and the
scan then syncs the last seven days. Only an explicit helper rejection of the
history-window field triggers the seven-day fallback. Settings retains a host
warning that the helper lists only seven days and needs an update for longer
history. A later accepted listing clears the warning. Timeouts, SSH failures,
and invalid listings remain scan errors and do not retry with seven days. A
session with no activity in the last seven days is not exported again while its
cached copy is intact and its listing entry (title, folder, surface, and
modification time) does not change. Directory-entry, candidate, preview-byte,
and elapsed-time budgets also bound discovery work, so a crowded store can
return a truncated result. A longer history has four times the entry, candidate,
and byte budgets, with the same time limit. A session missing from this bounded
list is not treated as deleted. Previously synced sessions remain readable
offline under the desktop's retention policy. The host row counts all saved
sessions, including sessions outside the discovery window; opening that list
keeps the Sessions date filter. A rejected session does not prevent other
sessions in the scan from syncing. The row keeps a warning with the number
rejected. A scan that completes and syncs at least one
session records a successful sync time, even when other sessions failed. A scan
in which every attempted session failed does not. The host row shows why a
failed scan stopped. An SSH failure at the first request (the hello step) shows
the same message as **Check connection**: "SSH could not reach this host", "SSH
key authentication failed for this host", "SSH could not verify this host key",
or a helper or platform message. When SSH reports "No route to host" for a
private IPv4 address (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), the message
adds that macOS can block local network access and names System Settings →
Privacy & Security → Local Network. The row never shows the raw SSH output;
the application log keeps it. Any other scan error shows "Some remote sessions
could not be synced", and partly failed scans show "N of M sessions could not
be synced". Each session export has a
no-progress limit of 60 seconds, which restarts whenever bytes arrive, and a
total limit of 30 minutes. A slow link that keeps sending does not time out
before the total limit. An export that receives no bytes for 60 seconds, runs
past 30 minutes, or whose remote stream breaks, fails only that session and counts as rejected. The scan stops after
three such failures in a row, because the host is then probably unreachable; a
successful or unchanged session resets the count. A session that fails to sync
writes one warning to the application log with the host id, agent, session id,
and a reason: `rejected_or_changed`, `timeout`, `transport` (SSH start, exit,
or stream failures), or `cache` (local write, limit, or import failures). The log
never contains paths, titles, transcript content, or SSH output. A slow link can
exceed the deadline: at about 0.4 MB/s, a 60 MB session takes 150 seconds and an
87 MB session takes more than 200 seconds, so the larger one times out. Saved counts update after
imports, deletion, and retention. SSH connection failures (including a connect
timeout), cancellation, a full cache, and local storage failures (including a
failed write of the transfer) stop the scan. Older helpers that cannot distinguish a session rejection also stop the
scan on failure. **Retry** requests another scan; a failed SSH connection does
not establish that the host is offline.

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
borrow this computer's agent configuration, provider account, or quota for
findings. Per-session findings remain subject to the copied evidence's own
completeness and source contract. Opted-in Cursor account usage is separate:
matched remote Cursor sessions can receive tokens and recorded cost from this
Mac's Cursor IDE login. See [Cursor account usage](session-coverage.md#cursor-account-usage).

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

Build on the target Linux architecture. For a Windows x64 host, build for the
`x86_64-pc-windows-gnu` target. From another platform, a cross linker such as
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) works:

```sh
cargo zigbuild --manifest-path crates/antiburn-remote/Cargo.toml --release --locked \
  --target x86_64-pc-windows-gnu
```

A build from source reports the version
in `crates/antiburn-remote/Cargo.toml`, which a release holds equal to the
application version. The release workflow produces static
musl binaries for both architectures, tests the native helper, rejects binaries
that require a dynamic loader or shared libraries, names each archive from that
same manifest, and includes the archives
in the application release's `SHA256SUMS`. Public-repository releases also include
build provenance; private-repository releases skip attestation. Running local builds
or tests does not create or publish a release.

# Privacy and data handling

antiburn (Mod) 0.1.1 reads native account quota data through the
Claude Code host. It uses percentages, reset timestamps, measurement status,
and the host clock to display the usage strip and its details.
Display state stays in host session atoms. The mod does not create its own
database, read transcripts or project files, inspect credentials, or export
account data.

The public artifact contains no live-check engine, activity observer, debug
previews, telemetry, updater, direct network client,
model calls, background timers, or prompt mutation. It does not inherit the
antiburn desktop app's analytics or optional third-party checks. There is no
bridge to the desktop app or a requirement to install the cross-vendor core.

Claude Code controls its own API requests, session storage, settings, logs,
plugin cache, and retention. Removing this mod does not by itself erase data
retained by the host. The host's own privacy terms and account policies still
apply. Activating the logo opens the fixed https://antiburn.com address. Desktop uses
a native link. Terminal activation reads the OS name and invokes a bounded
system opener; it displays the address if opening fails. Rendering and hover do
not launch a browser. Website visits use normal browser networking and send no
quota or session data through the mod. Documentation and support links also
open external websites.

The locally generated development variant can contain additional experimental
behavior and is not covered by the public artifact's capability description.
Do not use its behavior as evidence for this release's data handling.

For privacy questions, use the project's
[security reporting instructions](https://github.com/antiburn/antiburn/security/policy).
Do not attach credentials or private session content to public issues.

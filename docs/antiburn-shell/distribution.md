# Plugin distribution

The first unified candidate is **antiburn** for Claude Code. It is generated
locally as `plugins/antiburn`, with a separate local-only `antiburn-dev` build.
Codex, Cursor, OpenCode and Pi adapters remain unpublished future candidates.
The user-facing name remains antiburn; the internal package is
`antiburn-plugin-core`.

There are no real users of the old `antiburn-mod` identity. The 0.1.4 snapshot is
kept as a regression baseline, not as a production rollback obligation. A rename
map, legacy cache migration and independent recovery marketplace are unnecessary
for this initial launch. Fresh installs, current-version updates, removal and
public/dev independence still require validation.

The repository remains the source of truth. The [plugin page](../../plugins/README.md)
is its dedicated GitHub folder page. The `feat/antiburn-mod-release` testing
branch lists only public `antiburn` 0.2.0 from `plugins/antiburn`, pinned to a
reviewed immutable artifact commit. Install with the branch URL in the
[plugin guide](../../plugins/antiburn/README.md#distribution-status). The local
`antiburn-dev` build stays outside the hosted catalog. Its burn animation and
preview controls are temporarily disabled; live checks and actions remain.

This testing-branch update does not create a PR, release, draft or tag, and does
not update the main-branch catalog. Hosted install, update and reinstall checks
must verify the exact pinned files; local transport checks alone are insufficient.

## Later repository separation

A separate generated distribution repository can come later without moving the
source immediately. Export the same self-contained artifact, licenses, source
commit and hashes. Keep plugin identity and command/state namespaces stable;
verify fresh install, updates and removal against the exported bytes. Use one
authoritative generator and one destination per release. Leave a forwarding
README here after the new route is verified. Creating or publishing that
repository requires a separate request.

## Other agents

Keep host-specific catalogs separate: they do not share a schema. Codex remains
skills-only; Cursor uses its canonical `/antiburn` skill, replacing the old
owned findings skill; OpenCode separates its command asset from the optional
runtime; Pi retains its extension package. No listing, registry package,
Desktop compatibility or cross-vendor quota support is established by assembly.
Run the vendor's installation and native acceptance matrix when that vendor is
selected for release. Their certification is deferred from the Claude milestone.

Publishing, tags, uploads, marketplace changes exposed to users, listing
submissions, pushes and release drafts require separate authorization.

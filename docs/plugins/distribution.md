# Plugin distribution

The first unified candidate is **antiburn** for Claude Code. It is generated
locally as `plugins/antiburn`, with a separate local-only `antiburn-dev` build.
Codex, Cursor, OpenCode and Pi adapters remain unpublished future candidates.
The user-facing name remains antiburn; the internal package is
`antiburn-plugin-core`. For how sources, generated builds and catalogs connect,
see the [plugin architecture](architecture.md).

There are no real users of the old `antiburn-mod` identity. Its obsolete 0.1.4
snapshot is retained in Git history, not in the working tree. A rename map,
legacy cache migration and independent recovery marketplace are unnecessary
for this initial launch. Fresh installs, current-version updates, removal and
public/dev independence still require validation.

The repository remains the source of truth. The [plugin page](../../plugins/README.md)
is its dedicated GitHub folder page. The `feat/antiburn-mod-release` testing
branch lists only public `antiburn` 0.2.1 from `plugins/antiburn`, pinned to a
reviewed immutable artifact commit. This public version contains exactly 24 files;
the [Claude catalog](../../.claude-plugin/marketplace.json) records the authoritative
artifact SHA. Preserve those bytes under the same version. Install with the branch URL in the
[plugin guide](../../plugins/README.md#install). The local
`antiburn-dev` build stays outside the hosted catalog. Its burn animation and
preview controls are temporarily disabled; live checks and actions remain.

The testing branch does not create a release or tag. Main has no marketplace
catalog yet. Merging this catalog into main makes the pinned plugin available
through that route without a GitHub Release or tag, so catalog promotion needs
explicit distribution approval. A draft PR is a review checkpoint and does not
authorize promotion. Hosted fresh-install and reinstall checks must verify
the exact pinned files; local transport checks alone are insufficient.

## Public route after promotion

Merge the reviewed PR from `feat/antiburn-mod-release` into `main` only after
explicit approval. Public installs then add
`https://github.com/antiburn/antiburn.git` without a branch fragment. Main's
catalog selects the immutable artifact SHA; the PR branch is not the permanent
install source. The branch URL remains a temporary testing route until promotion.

The initial release has no users on older plugin versions. No older-version
upgrade or marketplace-source migration path is required. Internal testing
marketplaces are disposable test setup; they are not a supported user upgrade
route. Verify fresh installation from the permanent main route after merge.

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
skills-only; Cursor uses its canonical `/antiburn` skill; OpenCode separates
its command asset from the optional
runtime; Pi retains its extension package. No listing, registry package,
Desktop compatibility or cross-vendor quota support is established by assembly.
Run the vendor's installation and native acceptance matrix when that vendor is
selected for release. Their certification is deferred from the Claude milestone.

## Vendor rollout documentation checklist

Complete these documentation updates as part of each vendor rollout, before presenting its installation route as available:

- Validate the actual distribution source, package identity, host versions and native surfaces. Development runbook commands are not approved public installation instructions.
- Add the verified public installation, invocation, update and removal procedures to [installation.md](installation.md). Keep each vendor's prerequisites and limitations explicit.
- Update the supported-agent table in the [plugin page](../../plugins/README.md#supported-agents), the [compatibility record](compatibility.md), and the [validation scope](validation.md) with evidence for that vendor.
- Update this distribution guide with the approved channel and promotion requirements.
- Update the [architecture map](architecture.md) when the rollout changes source ownership, assembly inputs/outputs, adapter packaging or catalog boundaries.
- Remove or revise that vendor's unpublished label in [development-adapters.md](development-adapters.md). Keep contributor-only testing procedures separate and link to the public guide.
- Check relative links and examples, and confirm that unshipped vendors still carry an unpublished status. Include these documentation changes in the vendor's rollout PR.

Publishing, tags, uploads, marketplace changes exposed to users, listing
submissions, pushes and release drafts require separate authorization.

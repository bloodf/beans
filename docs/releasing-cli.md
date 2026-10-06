# Releasing Beans and the CLI

All platforms share stable GitHub releases in `bloodf/beans`, tagged `beans-v<version>`. The version is the root `package.json` version; the CLI and relay retain the Cargo workspace version, recorded separately in the release manifest. Drafts and prereleases are not update candidates.

## Release workflow

[`.github/workflows/release.yml`](../.github/workflows/release.yml) is manual-only. Dispatch an exact published stable `tag` with immutable `scope: server|all` selected before building. Server scope needs only the update signing key. All scope builds desktop installers and completed exact-source EAS APK, AAB and store IPA artifacts, with provenance. No published-release event starts an automatic server finalizer. A finalized server release cannot acquire clients; use a new version/tag.

See [release and account guide](architecture/releases.md) for EAS profiles, remote build-number authority, Expo linking, Apple/Google credentials, native push and separate manual Play internal/TestFlight submission. Store IPA is not an ad-hoc installer. Existing bytes and manifests remain immutable; retry uploads with original artifacts.

The finalizer collects only recognized staged distribution assets, never unpacked apps or raw executables. Server scope requires `beans-server-linux-x86_64.tar.gz`, `beans-server-linux-aarch64.tar.gz` and `beans-server-updater.py`. CLI archives are optional to the server manifest API, but each included CLI archive requires its matching `.sha256` file and the checksum must match its bytes; orphan checksums are rejected. The workflow includes both Linux CLI archives and checksums.

Before any upload, the finalizer verifies every existing GitHub asset's size and SHA-256 digest against the proposed inventory, including readiness bytes, and rejects unexpected assets. It uploads distribution assets, then `beans-update.json.sig`, then `beans-update.json` last. Existing bytes are never replaced. An interrupted finalization can resume only with identical artifacts, manifest and signature; a rebuild that changes bytes is not an identical retry. A finalized release cannot be extended or repaired in place. Release assets must not be manually changed after readiness is published.

`beans-update.json` contains schema 1, root version, exact 40-character revision, minimum relay protocol and artifact names, sizes, hashes, component/platform and component version. Its detached base64 Ed25519 signature covers the exact manifest bytes, including the final newline. Consumers derive download URLs from the fixed repository, validated stable tag and artifact basename; the manifest does not supply arbitrary URLs.

Scope changes the required inventory, not the manifest schema or trust key. Readiness is consumer-specific: the server updater requires its server archive and updater entry; Mac and Windows/Linux app updaters require their own complete platform assets as well as a valid signature. A signed server-only manifest does not authorize client installation; missing readiness, signature failure or missing consumer artifacts fails closed.

The public anchor is [`updates/public-key.txt`](../updates/public-key.txt). `BEANS_UPDATE_PRIVATE_KEY` supplies the corresponding Ed25519 PKCS8 PEM only to release jobs. MyGo receives base64 seed32+public32; Sparkle receives base64 seed32. There is no upstream key or login-keychain fallback. Keep the private key and Android keystore outside source and backed up; losing them prevents trusted future updates.

Server CI requires only `BEANS_UPDATE_PRIVATE_KEY`, matching the committed public anchor, and repository `contents: write` for finalization. All scope additionally needs Mac Developer ID/notarization credentials and linked Expo configuration with EAS-managed Android/iOS signing. Exact variables and legacy local signing inputs are in [Releases](architecture/releases.md#exact-credential-consumers). Installing secrets, creating accounts/credentials, cloud builds, publication and deployments are separate operator actions; pipeline source is not proof of shipped artifacts or live rollout.

### Source interfaces

`buildReleaseManifest(tag, revision, directory, scope = "server")` and `writeReleaseManifest(tag, revision, directory, scope = "server")` take `ReleaseScope = "server" | "all"`; `coreVersion` and `releaseProtocol` remain exported. The manifest builder checks inventory and CLI checksum consistency without signing; the writer signs the exact resulting bytes. With staged assets and the signing key supplied securely to the signing commands:

```text
bun run scripts/release-github.ts check <tag> [server|all]
bun run scripts/release-github.ts collect <source> <dir> [server|all]
bun run scripts/release-github.ts desktop <inventory> <dir>
bun run scripts/release-github.ts manifest <tag> <revision> <dir> [server|all]
bun run scripts/release-github.ts publish <tag> <revision> <dir> [server|all]
```

`collect` validates against the named distribution inventory; desktop staging reads the build's explicit asset inventory. `publish` requires GitHub CLI authentication with release-write access and rechecks the published release and exact tag revision. Do not finalize uncommitted or different-source builds by supplying the tag's revision string.

## CLI distribution

The website's [`install-cli.sh`](../web/public/install-cli.sh) and [`install-cli.ps1`](../web/public/install-cli.ps1) default to `https://github.com/bloodf/beans/releases` and use `beans-v` tags. Server workflow releases supply the Linux archives below; `all` supplies every listed platform:

```
lorca-cli-macos-aarch64.tar.gz
lorca-cli-linux-aarch64.tar.gz
lorca-cli-linux-x86_64.tar.gz
lorca-cli-windows-x86_64.zip
<archive>.sha256
```

The scripts choose the computer's OS/CPU, verify the adjacent checksum and replace the executable by rename. `LORCA_VERSION` selects a root release version; `lorca --version` reports the Cargo component version. `LORCA_INSTALL_DIR`, `LORCA_NO_MODIFY_PATH` and `LORCA_DOWNLOAD_URL` retain their existing installer meanings. An Intel Mac and Windows on Arm are not supplied. The standalone CLI checksums protect archive integrity; they are not the signed automatic-update readiness verifier.

When the latest release is server-only, Mac and Windows standalone CLI installers need `LORCA_VERSION` selecting a full release that actually contains their archive. The installers do not synthesize missing platform assets.

## Server and Runner updates

[`scripts/server-updater.py`](../scripts/server-updater.py) uses Python 3.9+, OpenSSL 3 and explicitly configured systemd services or Incus instances. [`updates/server`](../updates/server) contains an example config and a 15-minute timer. Installation does not enable the timer: an administrator reviews the root-owned config, token files, backup policy and service paths, then enables it explicitly.

On a Linux updater host, install from a trusted checkout with `sudo sh updates/server/install.sh`. This installs the updater, public anchor, service and timer, not a target configuration or enabled schedule. Create `/etc/beans/server-updater.json` from the example, owned by root with mode `0600`, naming the actual relay, Runner services/instances, binary/catalog paths, health endpoint, drain token files and backup policy. Ensure Python 3.9+, OpenSSL 3, systemd and any configured Incus access are available, and bootstrap old Runners with drain-capable binaries and matching service token configuration before unattended rollout.

After reviewing the configuration and backup/recovery procedure, run `sudo systemctl start beans-server-updater.service` and inspect its result before `sudo systemctl enable --now beans-server-updater.timer`. The timer starts after five minutes at boot and checks 15 minutes after the previous service finishes, with up to 60 seconds of randomized delay. Its installation alone, or release finalization alone, does not enable automatic server updates.

The updater verifies signed readiness and streamed asset hashes, extracts only `lorca`, `lorca-relay`, `models/v1.json` and `marketplace/v1.json`, and rejects links, traversal, unexpected archive entries and wrong-architecture executables. Root-owned private state serializes passes and records progress. A target already recorded as current is checked against its live process and version before it can be skipped.

The relay moves first. SQLite backup uses the database owner's privileges and SQLite's online backup API; an externally managed backup must be explicitly acknowledged in configuration. The relay must answer health with the expected component version and compatible protocol before any Runner moves.

A Runner service must already support `lorca update status|prepare|cancel` and set `LORCA_UPDATE_TOKEN_FILE`. Prepare closes new work admission under a lease while existing jobs finish. The updater renews the lease while waiting; expiry or cancel resumes work. Account Pause is unchanged, active jobs are not cancelled, and held relay envelopes keep their cursor position. A first installation therefore requires a controlled bootstrap upgrade, not an old Runner pretending to drain.

On both systemd and Incus targets, the installed executable is a regular root-owned file with executable bits, no write bits (including the owner's) and no set-id bits. Every ancestor directory is root-owned and not group/other-writable; symlink components are rejected explicitly before Incus metadata reads as well as by local `lstat`. The updater checks this before invoking the configured Runner binary, including status and cancellation. New and restored executables use root:root and mode `0555`, rather than inheriting ownership or writable/special permissions. Bootstrap the configured relay and Runner binaries with these permissions before the first pass. Catalog files and their complete directory ancestry are also root-owned and not group/other-writable. The example keeps catalogs under `/usr/local/share/beans-relay`, separate from the service-writable SQLite directory. Create the catalog subdirectories before rollout. The bootstrap installer checks its destination ancestry and existing files, installs the updater with mode `0555`, and leaves an existing config's ownership, permissions and bytes untouched; unsafe configuration requires explicit operator repair.

Before preparing a lease, the status endpoint's pid must match the exact configured unit's nonzero MainPID. Every prepare, status poll and renewal checks that same MainPID before and after the control request. Conservative lease expiry uses the request's monotonic start time and subtracts a second for lifetime rounding. The stop-start deadline separately reserves the full 300-second service-stop timeout; busy polling uses lease expiry, not that stop reserve. A final renewal must be ready under the same lease before the stop-start deadline. A different port's Runner cannot authorize or receive a drain/cancel for this service. A stopped Runner is not treated as idle from a `serving:false` response: automatic stopped-service recovery requires a retained journal for an authorized installation and an inactive/failed unit with MainPID zero. A stopped service without that journal requires operator review.

Files are staged on the target filesystem and renamed into place. Before any stop/swap, original binary/catalog bytes are synced into `<state_dir>/previous/<target>/<root-release-version>/`, and an atomically written, synced `journal.json` records their hashes, ownership, modes, destination paths, release inventory and service binding. Its `prepared` phase records complete originals, not a stopped service. For Runners, final drain confirmation precedes the stop. The updater persists `swapping` only after a verified service stop, immediately before file mutation or restart-only recovery. A stopped Runner with a journal still in `prepared` after an interruption requires operator review rather than inferring stop authorization from drain alone. Mixed interrupted recovery reuses those originals without deleting or replacing them; restart-only failure restores the same retained originals too. A fully swapped target without a healthy process and without a journal cannot invent its pre-release rollback state. Legacy retained files without a journal, incomplete original backups, mismatched journals and unfinished swaps from another release require manual recovery with those files preserved. The database is never automatically restored across potentially incompatible migrations.

`state.json` distinguishes release failures from operator configuration faults. A `Failure` during signature verification, manifest validation, hash checking, extraction, updater preflight or target installation records `failed` (the root release version) and `failed_reason`; that release is held until a newer release or explicit operator clearance. Authentication, missing drain bootstrap/token configuration, wrong-port/MainPID control, unsafe executable paths and recovery-journal configuration faults during a pass instead record `operator_hold` and exit 78. That hold stops further rollout, including newer releases, until the operator fixes the cause and clears it. Existing partial progress, original backups and the anti-downgrade `version_floor` remain intact. Missing published readiness and transient public-download failures remain temporary. A correctly configured Runner that simply stays busy cancels its lease and exits 75 for the existing idle polling on the next scheduled pass; cancellation/control failure becomes an operator hold instead. No attempt counter changes these classifications.

For manual recovery, stop the updater timer, inspect `state.json`, the affected service and retained journals, repair the reported fault and recover originals if needed. Clear only the relevant `failed`/`failed_reason` or `operator_hold` after reviewing that recovery; retain progress, version floors and original files. An interrupted pass without a hold resumes the partial rollout and rechecks the relay first. The signed updater replaces itself only after all targets succeed.

Platform-specific packaging and installation behavior are in [Mac releases](releasing-mac.md) and [Windows/Linux releases](releasing-desktop.md). Native Android installation requires OS approval. An ad-hoc IPA installs only on provisioned devices; GitHub cannot silently replace an installed native iOS app. Store submission runs separately through the manual testing-only workflow described in [Releases](architecture/releases.md#exact-credential-consumers).

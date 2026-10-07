# Beans project guide

Beans runs persistent bots on the user's Runners, with AppKit on macOS, MyGo on Windows/Linux and Expo over the Rust core on phones. Read [ARCHITECTURE.md](ARCHITECTURE.md) and its subject docs before changing a mechanism. Source provenance and project licensing are recorded in [README](README.md#license-and-credits); required third-party notices remain with their source.

## Fresh account format

The runtime binary is `beans`; owned crates/modules/packages use Beans names and configuration uses `BEANS_*`. Format is exactly `beans-v2`, with the same byte string as the identity/machine/push HKDF salt. Backups start with the separate token `beans-v2`, followed by thirteen base32 groups. Restore rejects a missing/wrong prefix before normalization or derivation. Pairing uses exact `beans://pair?v=2&…` links and validates the version, unique fields and sealed request/reply format before persistence.

Desktop homes are `~/.beans-v2` / `~/.beans-dev-v2`, with ports 4874 / 4875. Phone roots are `beans-v2/core` / `beans-dev-v2/core` within separate existing build sandboxes. AppKit preferences use `beans-v2.*` without old-key migration. Distribution/signing identifiers `ai.amoena.beans` / `.dev` and their app/keychain groups stay unchanged.

Device storage validates `format.json` before mkdir, permissions, deletion, recovery or SQLite setup; populated unmarked homes and malformed account records fail without modifying bytes. Direct LocalStore and push-decryption paths enforce the boundary. Device SQLite uses `BNS2` (`0x424E5332`). Relay SQLite separately requires `BNR2` (`0x424E5232`) before SQLite opens; Postgres requires exactly one `beans-v2` row in `beans_storage_format`, initialized only for a truly empty schema before account DDL/recovery. See [Identity](docs/architecture/identity.md#fresh-beans-format) and [Relay](docs/architecture/relay.md#fresh-relay-storage).

Protocol 5 plus `Beans-Protocol: 5` and `Beans-Format: beans-v2` gate every account route, including public registration/challenge/pair mailboxes and sync. Missing, malformed or duplicated capabilities fail closed. Effective account/roster floors cannot fall below 5. Old accounts, old unversioned backups and mixed-format paired Devices are incompatible. No automatic migration, reset or production rollout occurs; existing accounts, installed apps and services remain untouched.

## Preserved mechanisms

- Account Pause, per-bot shell/file/plugin capabilities, reviewed file drafts, policy reconciliation and roster CAS retain their existing behavior. Shell/allowed plugins still have the Runner user's authority; a workspace is not a sandbox and grants no automatic approval. See [Bots](docs/architecture/bots.md), [Tools](docs/architecture/tools.md) and [Protocols](docs/architecture/protocols.md).
- Blobatar appearance and uploaded photos remain independent, seeded by stable bot ids, with existing renderer/activity and durable roster behavior. Vendored Blobatar retains Alain's [MIT notice](packages/beans-blobatar/LICENSE). See [Avatars](docs/architecture/avatars.md).
- Provider credentials remain account-DEK-encrypted, shared through the `credentials` blob. Neutral custom-provider presets, discovery, defaults/references and capability handling remain unchanged. Production Grok OAuth fails before callback/network pending a verified Beans contract; loopback fixtures and API-key gateway models remain distinct. See [Providers](docs/architecture/providers.md).
- Memory service namespaces retain domain `beans.memory.v1`; fresh account public keys select new namespaces without reusing old banks. Consent, masking, queues, deletion fences, embeddings and the Lance Cloud transport block stay unchanged. See [Memory services](docs/architecture/memory-services.md).

## Relay and push configuration

Select an operator-supplied `BEANS_RELAY_URL`, saved relay, pairing relay or optional packaged `BEANS_DEFAULT_RELAY_URL`; development may select its LAN relay. There is no foreign production fallback or fabricated Beans hostname. Public model/marketplace feeds come from the selected relay or explicit overrides, with bundled offline catalogs.

APNs topics are `ai.amoena.beans` / `.dev`. Android prebuild requires a supplied `BEANS_GOOGLE_SERVICES_FILE` identifying the intended Beans app and matching relay Firebase service-account credentials. The Android-only plugin rejects missing/wrong-client inputs; iOS prebuild has no Firebase prerequisite. App/extension push keychain account is `beans-v2.push-key`, with no old-slot reads/deletes. No Firebase project or signing identity is invented. External signing inputs, Android keystore and Apple credentials remain outside source; see [Releases](docs/architecture/releases.md).

## Release and update contracts

New stable `beans-v<root version>` releases use Beans-only CLI/server archives, desktop installers and updater inventory. Existing published assets/manifests/signatures remain immutable. Manual `server|all` dispatch selects scope before building; server scope needs the existing Beans Ed25519 key, while all scope additionally coordinates signed desktop and exact-source EAS store builds. Finalization checks immutable bytes before uploads and publishes the detached signature before readiness. Publication, paid/cloud builds, store submission and live rollout are separate operator actions.

Signed readiness requires protocol >=5 and compatible `beans-v2` relay health/floors. Standalone CLI self-update stays unavailable even with `BEANS_SELF_UPDATE`; local `update status|prepare|cancel` and service contracts remain independent. AppKit provisions its native token only after core readiness and marker validation; its existing admission lease, drafts and guarded quit remain mandatory. Server updater bootstrap, target/token/backup configuration and timer enablement require explicit operator action against fresh-format targets. See [CLI releases](docs/releasing-cli.md), [Mac releases](docs/releasing-mac.md) and [Windows/Linux releases](docs/releasing-desktop.md).

## Authorship and acceptance

Generators own generated bindings/libraries; never hand-author compatibility stubs. A rename invalidates prior build receipts. After source freeze, the integration owner runs renamed offline gates, isolated fresh-home/old-marker rejection, backup/pairing/protocol fixtures and available native surfaces. No source receipt proves installation, live sync, runtime embedding assets, signed publication or deployment. Existing accounts and production services are not acceptance fixtures.

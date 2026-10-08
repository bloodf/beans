# Releases and mobile accounts

## Validation gates

Feature changes run relevant checks locally before integration: the affected Rust, Swift, desktop or phone suites, `bun test scripts/*.test.ts` for release scripts and workflow contracts, `bun run check:docs` for documentation, and `bun run check:catalog` for catalog and marketplace index changes. Feature PRs and pushes to `main` do not allocate runners for the Tests, Docs or Catalog workflows.

The release integration convention is a same-repository head branch named `release/**` targeting `main`, for example `release/1.0.15`. [Tests](../../.github/workflows/test.yml) runs the full platform matrix, [Docs](../../.github/workflows/docs.yml) checks architecture budgets and links, and [Catalog](../../.github/workflows/catalog.yml) checks model catalog and marketplace index update timestamps on these PRs, regardless of changed paths. Every validation job checks both repository identity and the `release/` head prefix before allocating a runner; fork PRs, including fork branches named `release/**`, are skipped. `scripts/release-pr-gates.test.ts` checks trigger scope and positive/negative job-gate cases across all three automatic validation workflows locally.

Release publication and mobile submission remain manual-only. E2E scripts remain manual/local and are not added to these validation workflows.

## Source and readiness

The all-platform preflight requires nonempty changelog notes matching the root version before builds start; desktop updater packaging consumes those notes.

[Release Beans](../../.github/workflows/release.yml) runs only by manual dispatch. Choose `server` or `all` **before** building against an already published stable `beans-v<root package.json version>` tag. Publishing a tag does not start a server finalizer. The checkout must match GitHub's exact tag commit; finalization rechecks it. Server scope needs only the Ed25519 update signing key and builds Linux x86_64/aarch64 CLI/server bundles. All scope adds CLI installation checks, Windows/Linux desktop installers, notarized arm64 Mac DMG/update ZIP and three completed EAS phone builds. Scope cannot change after readiness exists; use a new version/tag.

Every job that compiles the CLI, in [Tests](../../.github/workflows/test.yml) (Rust on Linux, macOS and Windows) and in this workflow (`core`, `desktop`, `mac`), installs `protoc@3.36.2` before its first compile with `taiki-e/install-action`, pinned to a full commit SHA, and `fallback: none`. The action checks the SHA-256 of the protobuf release archive and exports `PROTOC`. The Runner build needs it because `lancedb` pulls in `prost-build`, which runs `protoc` and does not bundle it; the phone and markdown crates do not reach it. The Windows test job also sets `CARGO_BUILD_JOBS=1`: with the Lance dependencies, parallel rustc processes exhausted memory on the hosted runner. `scripts/protoc-workflows.test.ts` fails if a compiling job lacks the step, installs it after the first compile, uses an unpinned action, pins a different version than the other workflow, or the Windows job loses that limit. Local builds use the `protoc` on `PATH` or `PROTOC`.

Distribution assets upload first, detached signature next, `beans-update.json` last. Schema 1 binds root version, revision, relay protocol and each asset's component, platform, version, SHA-256 and size. Existing assets must have identical bytes; unexpected or changed bytes fail before upload. Readiness cannot be repaired or extended. Retry interrupted uploads using original staged artifacts, not rebuilt desktop binaries. The server updater verifies readiness, upgrades the relay first, then drains Runners; see [Runner update drain](service-updates.md#runner-update-drain) and [CLI release operations](../releasing-cli.md).

Fresh-format release packaging is not account migration or live rollout. Device homes/ports and relay storage enforce `beans-v2` before mutation; old SQLite/PG state is rejected rather than recovered into the new format. Use explicitly fresh targets for separately authorized deployment, leaving existing accounts, installed apps and services untouched. See [relay storage](relay.md#fresh-relay-storage) and [identity format](identity.md#fresh-beans-format).


New signed releases require protocol at least 5. Before any Runner moves, including a recorded-installation recheck, relay health must report the expected component version, `format: "beans-v2"`, `protocol` at least the signed protocol, and integer effective `min_protocol` and `min_roster_protocol` between 5 and that signed protocol inclusive. Missing, malformed, old-format, too-low or unsupported evidence fails closed; lowering an operator setting does not admit old account writers.

GitHub normalizes the staged Windows installer name `Beans Setup <version>.exe` to `Beans.Setup.<version>.exe`. Publication resolves that exact transport name and verifies uploaded sizes and SHA-256 digests before publishing the readiness signature. The signed inventory retains the staged filename; the website accepts both spellings.

`release-build`, `release-publish` and `mobile-submit` are workflow environment names. Configure their access/reviewer policies deliberately before use; referencing a name does not install protection. Build, publication and store submission are distinct operator actions. The all-scope workflow invokes cloud builds and publishes GitHub artifacts, so dispatch it only after approving any build charges and publication. No workflow promotes to public stores. E2E remains outside automatic workflows.

Desktop inventory uses Beans-only Windows/Linux names for new releases; finalized historical assets/manifests are not renamed or rewritten. Windows installers carry Ed25519 updater integrity, **not Windows Authenticode publisher identity**. A trusted Windows publisher requires a code-signing certificate/service and separately implemented Authenticode packaging. Mac Developer ID/notarization and Ed25519 update signing serve different trust checks.

## EAS source and native build

`mobile/eas.json` explicitly uses EAS-managed remote signing credentials and remote application versions. EAS is the sole persistent mobile build-number authority. Initialize each platform above its existing store version using `eas build:version:set`; `autoIncrement` allocates once per build attempt. Failed attempts consume numbers; retries allocate new numbers. APK and AAB builds have distinct Android version codes. Do not also set timestamp numbers or use another store-number allocator. Local development config uses `1` without allocating store releases; legacy local/ad-hoc exports require explicit `BUILD_NUMBER` and are not store distribution.

Profiles:

- `production`: Android store AAB.
- `github`: store-signed Android APK, distributable on GitHub with OS installation approval.
- `testflight`: Apple store IPA, named `Beans-<version>-store.ipa`.
- `preview`: separate internal distribution, using EAS-managed credentials and remote versions.
- `development`: simulator development client.

A store IPA is not an ad-hoc installer and cannot generally be sideloaded from GitHub. Preview/ad-hoc iOS installs require provisioned devices and are not required by all-scope readiness. Existing legacy `release-ios` is independent and exports only ad-hoc/development distributions.

Run EAS from `mobile/`; the Git repository root is the upload context. Release profiles pin Bun `1.4.2` so EAS reads the checked-in lockfile format. Root `.easignore` excludes build output, native prebuild directories and credentials, retaining Cargo workspace/lock, scripts, root package metadata and `packages/beans-blobatar`. Do not upload only `mobile/`. Mobile `postinstall` runs `scripts/release-eas-native.ts` on EAS workers before Expo prebuild and CocoaPods; EAS's `eas-build-post-install` hook runs after pods and is too late. Rust is installed if missing, targets/cargo-ndk are prepared, then locked host and phone builds generate bindings/libraries. Host metadata uses `.so` on Linux and `.dylib` on macOS. Android accepts `ANDROID_NDK_HOME`/`ANDROID_NDK_ROOT`, or `ANDROID_HOME`/`ANDROID_SDK_ROOT` with `BEANS_ANDROID_NDK_VERSION`; EAS profile pins installed NDK `27.1.12297006`.

`release-eas.ts` waits for finished builds and validates exact revision, project, production application identifier, store distribution, profile, platform, root version and positive allocated number. It downloads from trusted HTTPS artifact hosts, validates every redirect against trusted HTTPS hosts, hashes bytes and writes `eas-<profile>.json` without token-bearing URLs. Signed inventory includes APK, AAB, store IPA and three provenance files. Pending/failed/wrong-source builds cannot publish readiness. EAS build IDs and numbers are retained per attempt. The helper accepts a completed retry build ID as its last argument; immutable publish retries still require every original artifact. Real EAS platform builds, binary signing/identities, installation and phone push need external acceptance; source tests do not prove them.

## Link Expo and signing accounts

No Expo project is prelinked. Obtain identifiers from your own account; absent values fail clearly rather than using dummy IDs.

1. Join/create your Expo organization/project deliberately in Expo dashboard. Creating a project/account is an operator action.
2. Set `BEANS_EXPO_OWNER` to account/organization slug and `BEANS_EXPO_PROJECT_ID` to that project's UUID. These are public identifiers. Set them locally before `eas project:info`, then check it identifies the intended existing project; `eas init --id <existing-project-uuid>` may link that existing project when deliberately requested. Keep env-based config authoritative.
3. Generate `EXPO_TOKEN` in Expo account settings and store it as a secret; grant only intended project access. Do not put it in `EXPO_PUBLIC_*` or commit it.
4. Configure matching public values in EAS production/preview environments too: owner, project UUID and `BEANS_APPLE_TEAM_ID`. GitHub env values resolve local configuration; they do not automatically become remote worker environment values. Android prebuild requires the operator Firebase file environment input matching the intended application id; iOS prebuild does not require Firebase.
5. Prepare signing interactively with `eas credentials --platform android` and `eas credentials --platform ios`. Retain existing Android upload/signing identity rather than silently generating a replacement. Register Apple app `ai.amoena.beans`, notification extension `ai.amoena.beans.notify`, and App Group `group.ai.amoena.beans`; authorize both targets. Check EAS app-extension credential discovery before cloud build. Set remote versions above existing store numbers.

Apple Developer Program membership is typically US$99/year, local currency/tax may differ. Team ID is a public ten-character identifier, mapped by `BEANS_APPLE_TEAM_ID` to Expo `ios.appleTeamId`. EAS remote credentials hold Apple Distribution certificate and app/extension profiles; these are distinct from Mac Developer ID, notarization and APNs credentials. A paid account/project does not authorize this code session to build or upload.

Google Play registration costs US$25 once; identity/device verification and account requirements apply. New personal accounts subject to testing requirements need a closed test with at least 12 continuously opted-in testers for 14 days before applying for production access. Internal testing does not satisfy that closed-test requirement. Complete Play App Signing and retain/upload the existing upload key. APK sideloads and Play installs must use compatible signing identities for upgrades; Play may sign delivered APKs with its app-signing key rather than your upload key. Explain this before mixing installation channels.

## Exact credential consumers

Use [safe environment example](../../mobile/.env.release.example); examples contain no real credentials. Workflow repository variables hold public identifiers; workflow secrets hold secret material. Do not add repository/EAS secrets or settings without operator approval.

**GitHub/desktop**:

- `BEANS_UPDATE_PRIVATE_KEY`: raw Ed25519 PKCS8 PEM, matching `updates/public-key.txt`; signs readiness/MyGo/Sparkle data.
- `BEANS_MAC_CERTIFICATE_P12`: base64 Developer ID Application certificate plus private key; `BEANS_MAC_CERTIFICATE_PASSWORD`: import password.
- `BEANS_APPLE_NOTARY_KEY_P8`: raw notarization `.p8`; `BEANS_APPLE_NOTARY_KEY_ID`, `BEANS_APPLE_ISSUER_ID`: notarization API identity. Local Mac alternatives: `SIGN_IDENTITY`, `NOTARY_PROFILE`, optional `SPARKLE_BIN`. Public builds leave `BEANS_DEFAULT_RELAY_URL` blank.
- `GH_TOKEN`: workflow token; publication job needs repository `contents: write`, other jobs read. Local helper uses authenticated GitHub CLI.

**Legacy local mobile signing, not EAS remote**:

Android `BEANS_ANDROID_ENV` names private mode-0600 env file; it supplies `BEANS_ANDROID_KEYSTORE` path, `BEANS_ANDROID_KEYSTORE_PASSWORD`, `BEANS_ANDROID_KEY_ALIAS`, `BEANS_ANDROID_KEY_PASSWORD`. Keystore also has mode 0600. Legacy GitHub transport names are `BEANS_ANDROID_KEYSTORE_BASE64` plus those three signing values; current EAS workflow does not consume them. EAS remote credential setup must import retained key deliberately. Legacy iOS transport values `BEANS_IOS_CERTIFICATE_P12`, `BEANS_IOS_CERTIFICATE_PASSWORD`, `BEANS_IOS_APP_PROFILE_BASE64`, `BEANS_IOS_NOTIFY_PROFILE_BASE64` are not consumed by current EAS workflow. Local iOS script uses `BEANS_IOS_APP_PROFILE_NAME`, `BEANS_IOS_NOTIFY_PROFILE_NAME`, `BEANS_IOS_EXPORT_OPTIONS`, optional `BEANS_IOS_SIGN_IDENTITY`, explicit `BUILD_NUMBER` and Apple team.

**Manual store submission**:

[Submit Beans mobile testing](../../.github/workflows/submit-mobile.yml) dispatch takes finalized tag/platform. It verifies signature, exact tag revision, signed provenance and downloaded store binary SHA-256 before submission. No `--latest`, auto-submit or public promotion. Android uses Play `internal`, `releaseStatus: completed`; iOS uploads to TestFlight, not App Store review. External TestFlight testing may need beta review; App Store release/review is a later manual action.

- `BEANS_ASC_APP_ID`: numeric App Store Connect application ID (public), not bundle ID.
- `BEANS_ASC_KEY_P8`: raw App Store Connect API private key; `BEANS_ASC_KEY_ID` and `BEANS_ASC_ISSUER_ID`: key/issuer identity. Helper writes a new private temporary mode-0600 `.p8` file; maps `ascApiKeyPath`, `ascApiKeyId`, `ascApiKeyIssuerId`, `ascAppId` to EAS submit config. Use a key/team role authorized for upload. This is not the notarization/APNs key purpose.
- `BEANS_PLAY_SERVICE_ACCOUNT_JSON`: authorized Google Play Developer API service-account JSON, written to a new private temporary mode-0600 file and consumed by `serviceAccountKeyPath`. Enable API, link/authorize intended Play app and grant least required releases permissions. First Play app upload may require manual Console upload before API submissions. This key is not Firebase server credentials.

`EXPO_ASC_*` and `EXPO_APPLE_TEAM_*` repair/build-signing overrides are not implemented by these helpers. Configure EAS-managed signing interactively; do not assume these variables configure submit, notarization or APNs. Submission validates/downloads artifacts before creating secrets, removes its private temporary credentials in `finally`, and restores exact original `eas.json` bytes on success/failure. Existing credential files are not touched.

## Native push and store readiness

Beans registers native tokens directly with relay; Expo account/project linking does not provision direct FCM/APNs.

- Android prebuild requires existing operator Firebase client JSON via `BEANS_GOOGLE_SERVICES_FILE` and validates the actual build id (`ai.amoena.beans` or `ai.amoena.beans.dev`). Missing, nonexistent, malformed or wrong-client inputs fail; no project/client is fabricated. Configure this as an EAS file environment variable for remote Android builders. Relay `BEANS_RELAY_FCM_SERVICE_ACCOUNT` supplies matching native FCM credentials, distinct from Play submission. iOS prebuild has no Firebase prerequisite.
- Apple APNs `.p8` is consumed by relay `BEANS_RELAY_APNS_KEY`; set `BEANS_RELAY_APNS_KEY_ID`, `BEANS_RELAY_APNS_TEAM_ID`, and `BEANS_RELAY_APNS_TOPIC=ai.amoena.beans`. App/extension entitlements and provisioning must permit production notifications and shared App Group. Validate real paired phone notification/decryption.
- Preserve persistent `BEANS_RELAY_SECRET`; changing it is not release setup. Server env/secret changes are separate operator actions.

Create store records/listings, screenshots, age/content ratings, privacy policy, Apple privacy disclosures and Google Data safety accurately. Complete export-compliance declarations, permission explanations, reviewer access and account-deletion requirements where applicable. Test signing, paired-device reachability, native push, retention and installed upgrades on real devices before claiming release acceptance.

## Official references

- [EAS npm hooks and ordering](https://docs.expo.dev/build-reference/npm-hooks/)
- [EAS monorepos](https://docs.expo.dev/build-reference/build-with-monorepos/)
- [EAS versions](https://docs.expo.dev/build-reference/app-versions/)
- [EAS configuration](https://docs.expo.dev/eas/json/)
- [EAS iOS submission](https://docs.expo.dev/submit/ios/)
- [EAS Android submission](https://docs.expo.dev/submit/android/)
- [Apple enrollment](https://developer.apple.com/programs/enroll/)
- [Google registration](https://support.google.com/googleplay/android-developer/answer/6112435)
- [Google personal-account tests](https://support.google.com/googleplay/android-developer/answer/14151465)

EAS artifact downloads follow HTTPS redirects only across the enumerated Expo API and artifact hosts, including `api.expo.dev` and `wf-artifacts.eascdn.net`. The downloader rejects credentials in URLs, unlisted hosts, redirect loops, oversized files, and non-ZIP payloads before recording artifact hashes.

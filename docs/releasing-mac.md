# Releasing the Mac app

Beans uses Sparkle with a project-owned Ed25519 anchor and GitHub assets from the [unified Beans release](releasing-cli.md). Root `package.json` supplies the version; the stable tag is `beans-v<version>` in `bloodf/beans`.

These files describe new Beans-only release assets. Published historical manifests and signatures remain immutable. Fresh-format builds do not migrate existing homes, preferences, accounts or unversioned backups; no installed app, production service or live relay rollout is changed by source packaging.


## Packaging prerequisites

Install Bun, the Rust/Swift toolchains, Xcode and `create-dmg`. SwiftPM resolves Sparkle's tools into `macos/.build/artifacts/sparkle/Sparkle/bin`; `SPARKLE_BIN` can select another installed tool directory.

Provide the matching `BEANS_UPDATE_PRIVATE_KEY` PEM, an installed **Developer ID Application** identity (`SIGN_IDENTITY` selects it), and a notarization keychain profile (`NOTARY_PROFILE`, default `BEANS_NOTARY`). Creating identities, storing Apple credentials and granting provisioning permissions are explicit operator actions, not release-script side effects. Public GitHub packaging refuses a private `BEANS_DEFAULT_RELAY_URL`.

```sh
bun run release-mac --local
```

This builds and signs Beans, checks its bundle version, creates/signs/notarizes the DMG, staples the DMG and application, verifies code signing and Gatekeeper acceptance, then archives the stapled application as `Beans-<version>.zip`. `dist/mac` contains the DMG and `updates/` contains the ZIP and signed `appcast.xml`. No upload or version bump occurs. A Finder-customization warning is not accepted as notarization proof: signing and assessment still must succeed.

The appcast generator requires exactly the current Beans ZIP, uses seed32 on stdin, embeds the release protocol derived from the same constants as the readiness manifest, and signs the final modified XML. Deltas and upstream release history are disabled. The unified workflow hashes these final bytes and publishes readiness only after every platform succeeds.

## Installed updater

`scripts/app.ts` writes the Beans feed/public key into release bundle metadata. Debug bundles omit them and disable checks. `Updater.swift` enables Sparkle only for the Beans production identity and feed; saved opt-outs are preserved. Automatic checks use Sparkle's minimum one-hour interval.

The native updater verifies signed readiness, requires protocol at least 5 and pins the appcast to that exact release, archive URL/size, version and required protocol. At installation it fetches readiness again and requires identical signed manifest bytes. Ephemeral relay health requests carry no account credentials or cookies. The selected relay must answer `/v1/health` with `ok: true`, service `beans-relay`, `format: "beans-v2"`, protocol at least the signed requirement, and integer effective `min_protocol`/`min_roster_protocol` from 5 through that requirement. Missing, malformed, wrong-format or unsupported evidence fails closed.

### Atomic Runner admission drain

The native launcher fixes Beans to `~/.beans-v2` / `4874` and Beans Dev to `~/.beans-dev-v2` / `4875`. Preferences use `beans-v2.*` without old-key migration; a saved different port fails closed. It passes the fixed `native-update-token` path into the child environment without creating storage. Only after core readiness does it validate the existing `beans-v2` marker and provision/read the private token through no-follow descriptors. Exclusive 0600 creation preserves existing token bytes and permissions; bounded regular-file and ownership/access checks remain mandatory.
The opened home and token must belong to root or the current user. The home is not group/other writable; the token is a bounded regular file with no group/other access. Validation never replaces its bytes or changes existing permissions.

An inherited `BEANS_UPDATE_TOKEN_FILE` disable/operator setting cannot authorize native installation. The app does not read, replace or modify an operator token or substitute it for app-owned trust. Installation needs validated app-owned control and a drain-capable connected CLI with the matching token; ordinary app requests do not imply permission to install.

Automatic installation waits for normal quit. Retained composer drafts (including behind Settings), active work, command input, onboarding, modal windows, sheets and edited documents block installation. The app does not clear drafts, cancel jobs, stop commands or change account Pause to make an update ready.

The guarded quit takes a token-authenticated `update.prepare` lease with a 1,800-second TTL **before** inspecting Runner work or fetching readiness and relay health. The CLI closes new admission under its work-producer lock; admitted turns, rooms, routines, command continuations and remote waits remain intact. If work is still admitted, the app releases the lease and cancels quit rather than forcing a stop. Relay envelopes held by the CLI drain remain queued for replay when admission resumes.

While admission is closed, the app rechecks the signed release, the bootstrap snapshot, actual selected-relay health and a subsequent `hello` relay selection. A final `update.status` must show the same Runner process, an enabled, prepared, ready control, no active work or jobs and positive remaining lifetime. Lease renewals retain the same lease id and process. Authorization also checks a conservative monotonic deadline measured from before the prepare request, the unchanged configured CLI port, the current native relay selection and the native quit guard immediately before use. An expired, canceled or replaced lease cannot authorize installation.

A user-requested Sparkle relaunch enters the same AppKit quit path. Its continuation is retained as a main-actor closure and invoked synchronously only after validation, after ending the first deferred AppKit quit request. The lease stays held through this deliberate handoff, and Sparkle's subsequent quit rechecks readiness, relay health and native safety again. Failed or canceled quits and finished/aborted update cycles release the native lease through `update.cancel`; cancellation waits for any in-flight prepare response before sending the release. A continuation that does not reach another guarded quit within 30 seconds also releases the lease. If release cannot reach the CLI, the error is surfaced and installation remains unauthorized; the CLI lease still expires independently. Subsequent explicit quits can try again without losing the postponed continuation.

Installed-app quit/relaunch behavior still requires real UI verification. Sparkle signing-tool proof and Swift compilation do not establish installed-update acceptance.

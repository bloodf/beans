# Mobile release trust

A mobile release binary is staged and published only after its native identity and its signer pass an explicitly approved, public trust policy. A candidate's own certificate, EAS metadata or a build record never establishes trust. Code lives in `scripts/release-trust.ts` (policy, preflight, signer gate, verifier), `scripts/release-native-identity.ts` (identity from one admitted snapshot) and `scripts/release-ipa-claims.ts` (IPA signed claims); the staging and manifest callers are `scripts/release-eas.ts` and `scripts/release-github.ts`. [Releases](releases.md) describes the pipeline around it.

## Public trust policy

`--public-trust-policy <file>` is always the final argument pair. The file is at most 64 KiB of JSON mapping `github`, `production` and `testflight` (no other key) to one policy each:

- `github` (APK): `{ profile, signerSha256: [pins], rotation: "none" }`, the exact current signer set.
- `production` (AAB): `{ profile, uploadSignerSha256: pin, rotation: "none" }`, the upload signer. It is not the Play app-signing identity, which Play holds.
- `testflight` (IPA): `{ profile, teamId, certificateSha256: [pins] }`.

Pins are lowercase 64-hex SHA-256 certificate digests, unique and nonempty, and a policy object carries no other key. No policy ships in the repository; the release workflow reads it from the `BEANS_PUBLIC_TRUST_POLICY` repository variable.

## Admission order

`release-eas.ts build` and its local mode validate the profile's policy and find every inspection tool on `PATH` (APK: `unzip`, `aapt2`, `apksigner`, `java`; AAB: `unzip`, `bundletool`, `jarsigner`, `keytool`, `java`) before reading credentials, checking the checkout, calling EAS or starting a build. `release-github.ts check`, `manifest` and `publish` admit all three policies for `all` scope before release notes, the private update key or network work; `server` scope needs none.

## Signer gate

After identity matches `ai.amoena.beans`, the release version and the build's own number, the signer gate runs on the same admitted snapshot:

- APK: `apksigner verify --verbose --print-certs` must report exactly one `Number of signers` line and that many ordered `Signer #N certificate SHA-256 digest` records, and the set must equal the pinned set exactly: no extra, missing or duplicate signer.
- AAB: `jarsigner -verify` must report `jar verified.` with no unsigned entries, since it can exit zero for an unsigned archive; `keytool -printcert -rfc -jarfile` must yield exactly the pinned upload certificate. Several signers or chains are refused, not guessed.
- IPA: plist identity is read, and on macOS `codesign --verify --strict --deep` and the signed entitlement claims (team, application identifier, groups, no debugger allowance) are checked. These are integrity facts, not Apple distribution authorization, so the gate then refuses every IPA.

Tools run with argument vectors, a fixed `C` locale, only `PATH` and `JAVA_HOME`, a 30-second limit and bounded output. A missing tool, a nonzero exit or malformed output refuses.

## Where it applies

Cloud staging verifies the downloaded bytes before writing the artifact or its proof; local exact-source staging verifies the retained snapshot. `buildReleaseManifest` re-reads every mobile binary of an `all`-scope inventory, checks its hash against the proof and verifies it again, so a swap after staging is refused. Without a verifier, an `all` manifest refuses.

Because Apple authorization (provisioning profile, team and chain validation) is not implemented, `testflight` staging and every `all`-scope check, manifest and publication fail closed with `IPA cryptographic trust verification unsupported`, even with a complete policy. Server-scope releases are unaffected.

## Tests

`scripts/release-trust.test.ts` covers policy shapes, argument handling, preflight order, signer enumeration, caller refusal before EAS or builds, IPA claims and a real JDK-signed AAB fixture. Its signed-APK fixture needs Android build-tools 34.0.0; without them it is skipped locally and required in CI (`scripts/android-fixture.ts`).

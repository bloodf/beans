# Releasing the Windows and Linux app

The desktop keeps the runtime identity Beans (`app.beans`) and uses the root `package.json` version. Distribution belongs to the [unified Beans release](releasing-cli.md) in `bloodf/beans`, tagged `beans-v<version>`, not a separate desktop release.

New release files use Beans-only package/archive/installer names; published historical manifests and assets are not renamed. Fresh Device storage uses `.beans-v2` / `.beans-dev-v2` and ports 4874 / 4875. Protocol-5 readiness and `beans-v2` relay health with supported account/roster floors >=5 are required. These sources neither migrate old accounts nor install over production apps/services.


## Build and staging

```sh
bun scripts/desktop.ts release linux/amd64 --stage-only
bun scripts/desktop.ts release linux/arm64 --stage-only
bun scripts/desktop.ts release windows/amd64 --stage-only
```

The script builds the bundled CLI and invokes MyGo with the project-owned Ed25519 signing key. It records exact distribution paths in `desktop/build/release-assets.json`; the unified workflow consumes that inventory rather than uploading unpacked application files. Archives are `beans-<version>-<platform>.tar.gz`, metadata is `update-<platform>.json`, and the Windows installer is `Beans Setup <version>.exe`. Linux also produces `beans_<version>_<arch>.deb` and install scripts published as `install-linux-amd64.sh` / `install-linux-arm64.sh`. Deltas are disabled. Non-staging manual upload requires a parent-created Beans draft and a matching root changelog section.

`BEANS_UPDATE_PRIVATE_KEY` is the release PEM. The signing helper checks it against `updates/public-key.txt` and converts it to MyGo's seed32+public32 format. Release signing never falls back to an upstream key or a MyGo configuration-folder key.

## Update eligibility

[`desktop/updater.go`](../desktop/updater.go) discovers stable Beans releases, verifies signed readiness and binds the selected archive's SHA-256 to MyGo's archive verification. A context-bound HTTP transport supplies pinned metadata to MyGo v0.1.22 while unrelated requests retain their original transport. Archive requests accept the exact signed URL and GitHub asset-storage redirects whose originating request is that archive, not arbitrary storage-host downloads. Public requests strip cookies and authorization. A new SDK version requires checking that interface again.


The independent selected-relay health recheck carries protocol-5/`beans-v2` headers and requires the fresh format, supported effective account/roster floors >=5, at least the signed required protocol and memory-config capability 1. Duplicate or malformed format/capability evidence cannot authorize installation. New native/browser preference namespaces do not migrate old settings or ports.

Automatic checks run every 15 minutes when enabled. Saved check/download preferences, skipped versions and last-check time remain in `updater.json`; development and non-writable installations do not update. Users can choose Install on Quit, Later or Skip. The app never forces an unattended relaunch. Native dialog words come from the existing frontend `L()` lookup, with its ordinary English fallback for untranslated keys.

Installation requires a drain-capable CLI child owned by this app. An external service, an old CLI without update control, or an unavailable/private-token failure defers installation without terminating work. In update-enabled release builds the launcher provisions `<BEANS_HOME>/update-token` (the normal CLI home when unset) using exclusive creation and random bytes; it never truncates an existing token. Unix creation uses mode 0600. Windows creation uses the system PowerShell to remove inherited access and grant only the current user's SID before writing the secret; unavailable ACL setup fails without writing a token. An inherited `BEANS_UPDATE_TOKEN_FILE`, including an explicit empty value, is preserved without provisioning a substitute. Operator files must already be private regular files, including their Windows ACL. Development and non-writable builds do not provision a token, and their imported environment remains intact.

Normal quit first rejects running work, retained composer drafts, editing sheets and extra settings/onboarding windows, then makes the page inert while validating the relay. A dedicated non-reconnecting local websocket pins update control to the child PID, launcher generation and selected port. The app refuses an already-prepared lease, requests a 30-minute admission lease, and requires ready/prepared status with at least ten minutes remaining. New Runner work waits atomically behind the lease while admitted work is left untouched. A second snapshot checks relay selection and drafts; the app checks lease readiness and CLI identity again before approving quit. This control is serialized by the app's single-instance lock; an operator must not run a competing updater against that Runner.

Failed preparation, port/reconnect/relay changes, preference cancellation and vetoed quit release the original Runner's lease through that pinned socket and restore page input. MyGo exposes no canceled-quit event, so a one-second watchdog resets approval when termination has not begun. If cancellation cannot reach the Runner, the lease expires and resumes admission; account Pause is never changed.

The combined quit callback stops the local app connection and waits for the approved idle child to exit before installation, while the single-instance lock is still held. Ordinary Windows quit retains parent-exit shutdown; approved update quit stops the idle CLI explicitly so resource locks do not wait for an app exit that installation would delay. MyGo renames the running Windows app executable aside and cleans it up on the next user launch. Readiness and selected-relay health are revalidated before installing; installation failures are logged and MyGo attempts its existing file-swap rollback. The app does not claim database rollback or restart itself. Real platform shutdown, ACL, resource-lock and rollback behavior requires parent verification; compilation alone is insufficient.

On Windows, the per-user installer location can update itself. On Linux, the install script's `~/.local/beans.app` can update itself; a package-managed `/opt` installation is updated through the package manager. The public download page links only to releases carrying readiness assets. No update request carries account credentials.

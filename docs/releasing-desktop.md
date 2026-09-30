# Releasing the Windows and Linux app

The app in `desktop/` updates itself through MyGo's updater plugin. Releases live in the Cloudflare
R2 bucket `lorca-releases`, served at `https://releases.lorca.app`: for each platform the installer
(`Lorca Setup <version>.exe`, the Debian package), the app as an archive
(`lorca-<version>-windows-amd64.tar.gz`), delta updates from the last three versions, the Linux
`install.sh`, and `update-<platform>.json`, the manifest the app checks. Installed apps accept only
archives and deltas signed with the update key, whose public half is `updates.publicKey` in
[`desktop/mygo.config.ts`](../desktop/mygo.config.ts). One command builds and publishes:

```sh
bun run release-desktop
```

- Updater: [`desktop/updater.go`](../desktop/updater.go). **Check for Updates…** in File and Help,
  and the Updates rows in Settings › General.
- Configuration: `updates` in [`desktop/mygo.config.ts`](../desktop/mygo.config.ts).
- Release: [`scripts/desktop.ts`](../scripts/desktop.ts), which runs `mygo build -upload`. MyGo's
  [auto-updates guide](https://github.com/egoist/mygo/blob/main/docs/updates.md) covers what it
  signs and uploads.

## One-time setup

### 1. Update key

`mygo keygen` (run in `desktop/`) writes the key pair to MyGo's folder in the user's configuration
directory: `%APPDATA%\mygo\update-keys` on Windows, `~/Library/Application Support/mygo/update-keys`
on macOS, `~/.config/mygo/update-keys` on Linux. `mygo-update.pub` is `updates.publicKey`;
`mygo-update.key` is the secret. Keep a copy in a password manager, and put it in that folder on
every computer that releases, or in `MYGO_UPDATER_PRIVATE_KEY`, which `release-desktop` prefers.

Without the secret key, no install in the field can be updated again.

### 2. R2 bucket and domain

1. The bucket is `lorca-releases`, with the custom domain `releases.lorca.app` (R2 ▸ the bucket ▸
   Settings ▸ Custom Domains). Objects are public at `https://releases.lorca.app/<file>`.
2. Create an R2 API token with Object Read & Write on that bucket. Its S3 credentials are the
   access key ID and the secret access key.

### 3. Environment

`release-desktop` needs:

| Env | Purpose |
| --- | --- |
| `R2_ACCOUNT_ID` | the Cloudflare account, for the endpoint `https://<id>.r2.cloudflarestorage.com` |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | the R2 token's S3 credentials |
| `MYGO_UPDATER_PRIVATE_KEY` | the secret key itself, when `mygo-update.key` is not in MyGo's folder |

It stops before building when one is missing.

## Cutting a release

The version is `"version"` in the root [`package.json`](../package.json), the Mac app's too.

1. Make sure [`CHANGELOG.md`](../CHANGELOG.md) has a `## [<version>]` section: it becomes the
   release notes of the update window (`updates.changelog`), and `release-desktop` stops before
   building without it. A version the Mac app released already has one.
2. Run:

   ```sh
   bun run release-desktop [platforms]
   ```

Platforms are MyGo's, comma separated. The default is this computer's, or `linux/amd64` and
`windows/amd64` from a Mac. The script:

1. refuses a version already published for any of the platforms, unless `FORCE=1`;
2. builds the CLI for each platform into `desktop/resources/<goos>-<goarch>/bin`, as
   `bun run desktop:build` does;
3. runs `mygo build -platform … -upload`, which builds the apps and installers into
   `desktop/build`, reads the published manifests and archives to make delta updates, signs the
   archives and deltas, and uploads everything to the bucket, the manifests last with
   `Cache-Control: no-cache`, which publishes the update of each platform.

To test an update, install an older release with its installer and choose **Check for Updates…**.

## Notes

- **Where apps update.** On Windows, the per-user install of the installer, in
  `%LOCALAPPDATA%\Programs`. On Linux, the install of
  `curl -fsSL https://releases.lorca.app/install.sh | sh`, which puts the latest version in
  `~/.local/lorca.app`. The Debian package installs in `/opt`, where the app cannot write: it
  leaves the menu item and the Settings rows out, and the next package updates it.
- **A development build never updates.** `mygo dev`'s Lorca Dev leaves the menu item and the
  Settings rows out.
- **Old archives stay in the bucket**, so the next release can make deltas from them.

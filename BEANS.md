# Beans fork

Beans is a Lorca fork with Beans branding on macOS and phones and project-owned changes to avatars, bot controls, relay compatibility, local websocket access, and custom-provider model discovery. The Swift target, Rust crates, `lorca` binary, `LORCA_*` environment variables, and pairing protocol (`lorca://pair`) retain their upstream runtime identifiers. The macOS bundles keep their CLI identities separate from upstream Lorca: Beans uses `~/.beans` on port `4864`; Beans Dev uses `~/.beans-dev` on port `4865`. The Windows and Linux app retains its upstream identity while sharing functional UI changes.

## Differences from upstream

The table below tracks branding and build configuration; it is not an exhaustive inventory of fork changes. The project-owned [`@beans/blobatar` package](packages/beans-blobatar/README.md) generates matching offline bot avatars across Mac, phone, and Windows/Linux. Its vendored Blobatar source retains Alain's MIT attribution and license in [`packages/beans-blobatar/LICENSE`](packages/beans-blobatar/LICENSE); bot images uploaded by users override the generated look.

Account-wide Pause, per-bot shell/file/plugin capabilities, and user-approved file drafts change bot behavior, not just copy. Pause and capability edits sync as encrypted `policy` actions alongside roster state so stale offline roster uploads do not undo explicit restrictions. Runner enforcement begins when a Device receives those actions; an offline Runner cannot enforce a change it has not received. Local CLI websocket upgrades check loopback `Host` and restrict browser `Origin` to the CLI's own origin or exact configured origins. These are source-tree mechanisms, not claims that a relay or client build has been deployed. See [Bots](docs/architecture/bots.md), [Tools](docs/architecture/tools.md), [Protocols](docs/architecture/protocols.md), and [CLI runtime](docs/architecture/runtime.md).

Shell Auto-review has no implicit workspace exemption. Only statically read-only commands skip review while Auto-review is enabled; mutations and opaque programs use the existing review and permission flow even inside a bot's workspace. Explicit user rules still apply, and an unavailable review fails closed for unattended work. See [Tools](docs/architecture/tools.md).

Relay protocol 3 carries durable encrypted policy actions and conditional roster writes. Upgrade every relay replica first, then clients: the CLI refuses relays older than protocol 3, and a relay with minimum protocol 3 returns HTTP `426` to older clients on `/v1` routes (except health). If deployment overrides `LORCA_RELAY_MIN_PROTOCOL`, set it to 3. Mac, phone, and Windows/Linux UIs expose account and bot controls and refresh saved custom-provider model lists without replacing selected models; the CLI also refreshes catalogs periodically. Runner network restrictions are deployment configuration, not a guarantee provided by the application. See [Providers](docs/architecture/providers.md) and [Codemode and Plugins](docs/architecture/plugins.md).

| File | Why |
| --- | --- |
| `scripts/app.ts` | Names the macOS bundles Beans / Beans Dev, assigns `ai.amoena.beans` / `.dev`, copies the unchanged Swift `Lorca` executable under the Beans bundle name, and omits the upstream Sparkle feed and key while disabling automatic checks. |
| `package.json` | Uses the Beans Dev APNs topic for the standalone local relay script. |
| `scripts/android-release.ts` | Rebuilds the Android core and generates a production release APK with external signing credentials. |
| `scripts/dev.ts` | Uses the Beans Dev APNs topic for the local relay and reports its CLI port. |
| `scripts/reset.ts` | Limits macOS resets to the selected Beans CLI home and port. |
| `scripts/mobile.ts` | Opens the Beans Dev phone app and its Expo development URL. |
| `scripts/release-ios.ts` | Uses the Beans iOS bundle ID and generated Xcode project, scheme, and archive paths. |
| `macos/Sources/Lorca/App/AppInfo.swift` | Recognizes the Beans Dev bundle, displays Beans when bundle metadata is missing, sets separate CLI homes and ports, and sets the Beans release relay fallback. |
| `macos/Sources/Lorca/App/Updater.swift` | Never starts Sparkle or exposes update controls without a Beans appcast. |
| `macos/Sources/Lorca/App/AppDelegate.swift` | Names Beans in the relay update warning and product note. |
| `macos/Sources/Lorca/App/MainMenu.swift` | Names Beans in the app and Help menus. |
| `macos/Sources/Lorca/Chat/ChatViewController.swift` | Names Beans as the system message author. |
| `macos/Sources/Lorca/Chat/Dictation.swift` | Names Beans in permission errors. |
| `macos/Sources/Lorca/Content/StateViewControllers.swift` | Names Beans in the CLI unavailable state. |
| `macos/Sources/Lorca/Model/CLIClient.swift` | Names Beans in the CLI connection error. |
| `macos/Sources/Lorca/Onboarding/OnboardingWindowController.swift` | Names Beans in onboarding copy. |
| `macos/Sources/Lorca/Settings/AutoReviewSettingsViewController.swift` | Names Beans in Auto-review copy. |
| `macos/Sources/Lorca/Settings/DeviceSettingsViewControllers.swift` | Names Beans in relay status. |
| `macos/Sources/Lorca/Settings/SettingsWindowController.swift` | Names Beans in app settings. |
| `macos/Sources/Lorca/Sheets/CustomProviderViewController.swift` | Names Beans in provider setup. |
| `macos/Resources/en.lproj/InfoPlist.strings` | Names Beans in English system permissions. |
| `macos/Resources/zh-Hans.lproj/InfoPlist.strings` | Names Beans in Chinese system permissions. |
| `macos/Resources/zh-Hans.lproj/Localizable.strings` | Translates Beans-facing UI strings. |
| `mobile/app.config.ts` | Names both phone builds, separates IDs/groups/schemes, and reads Android Firebase configuration only from `BEANS_GOOGLE_SERVICES_FILE` when set. |
| `mobile/plugins/with-android-release-signing.js` | Applies the external release keystore during Android prebuild without changing debug signing. |
| `mobile/plugins/with-android-locale-defaults.js` | Gives Android permission translations a default English resource for release lint. |
| `mobile/locales/en.json` | Names Beans in English system permissions. |
| `mobile/locales/zh-Hans.json` | Names Beans in Chinese system permissions. |
| `mobile/app/pair.tsx` | Recognizes Beans Dev and names Beans in pairing copy. |
| `mobile/app/settings/index.tsx` | Names Beans in settings and version fallback. |
| `mobile/app/settings/custom-provider.tsx` | Names Beans in provider setup. |
| `mobile/app/settings/device/[id].tsx` | Names Beans in relay status. |
| `mobile/src/i18n/zh.ts` | Translates Beans-facing phone strings. |
| `mobile/src/core/pairing.ts` | Names Beans in invalid pairing-code errors; still parses `lorca://pair`. |
| `mobile/src/core/core.test.ts` | Tracks the pairing error wording. |
| `mobile/src/core/model.ts` | Names Beans in product-facing model descriptions. |
| `mobile/src/core/prefs.ts` | Recognizes the Beans Dev ID while keeping the core's existing data-folder names. |
| `mobile/src/ui/ChatsScreen.tsx` | Names Beans in relay-update status. |
| `mobile/src/ui/Composer.tsx` | Names Beans in permission prompts. |
| `mobile/src/ui/relay.ts` | Names Beans in connection errors. |
| `mobile/modules/lorca-core/ios/LorcaCoreModule.swift` | Shares the push key through the Beans app group. |
| `mobile/modules/lorca-core/android/src/main/java/app/lorca/core/PushService.kt` | Selects the Beans Dev sandbox for push decryption. |
| `mobile/targets/notify/NotificationService.swift` | Reads the push key from the matching Beans app group. |
| `ARCHITECTURE.md` | Records the platform install identities. |
| `docs/architecture/macos-app.md` | Describes the bundle, CLI home and port, log location, and disabled updater. |
| `docs/architecture/phone-app.md` | Describes phone identifiers, push group, and optional Firebase setup. |

The Rust core emits `lorca://pair` codes. Beans registers `lorca` as a secondary URL scheme alongside its primary `beans` / `beans-dev` scheme so a link from another Device can open pairing. If upstream Lorca or both Beans variants are installed, OS selection of the `lorca` link handler is not deterministic; scanning or pasting the code inside the intended app still works. Android push requires a Beans Firebase client file; without `BEANS_GOOGLE_SERVICES_FILE`, Firebase push configuration is omitted.

## Relay selection

Release packaging in `scripts/app.ts` embeds an optional `LORCA_DEFAULT_RELAY_URL` as `BeansRelayURL` bundle metadata. `macos/Sources/Lorca/App/AppInfo.swift` reads that value; `CLILauncher.swift` passes it to the CLI only when the launch environment does not already set a fallback. Development bundles omit it. A release built without the variable has no preset relay. The phone inherits the relay URL in its pairing code. `crates/cli/src/config.rs` and `crates/cli/src/app.rs` resolve `LORCA_RELAY_URL` first, then saved settings, the paired Device URL, the development LAN relay, and finally `LORCA_DEFAULT_RELAY_URL`. Configure your relay explicitly before using the fork. Keep private build values outside source and contribution notes.

## Production notes

When APNs is used, run the relay with `LORCA_RELAY_APNS_TOPIC=ai.amoena.beans`. `scripts/release-mac.ts` and the publishing constants in `scripts/app.ts` (`RELEASES_URL`, `FEED_URL`, `SPARKLE_PUBLIC_KEY`) still name upstream release hosting and are not used by Beans builds. `mobile/google-services.json` is upstream's and is not used unless `BEANS_GOOGLE_SERVICES_FILE` points to a Beans file.

## Android release build

Run `bun run android:release` from the repository root. It rebuilds the Rust phone core, runs a clean production Android prebuild, and assembles `mobile/android/app/build/outputs/apk/release/app-release.apk`. Expo SDK 57 uses JDK 17; set `JAVA_HOME` for the command if your shell uses another JDK.

The signing keystore lives outside the repository at `~/.config/beans/android/beans-release.keystore`. The script reads `~/.config/beans/android/keystore.env`, or the path in `BEANS_ANDROID_ENV`; both that file and the keystore must have mode 0600. The env file must supply the complete set of `BEANS_ANDROID_KEYSTORE`, `BEANS_ANDROID_KEYSTORE_PASSWORD`, `BEANS_ANDROID_KEY_ALIAS`, and `BEANS_ANDROID_KEY_PASSWORD`; inherited shell values cannot fill missing entries. Back up the keystore and passwords to keep future APK updates installable. Signing configuration is added only when these variables are set, by `mobile/plugins/with-android-release-signing.js` during prebuild. Without `BEANS_GOOGLE_SERVICES_FILE` pointing to a Beans Firebase client file, the Android build omits Firebase push configuration and push is unavailable.

## Merge upstream

On branch `beans`, run `git fetch upstream` then `git merge upstream/main`. Resolve branding and build conflicts in the table above while keeping Beans IDs, app groups, isolated macOS CLI homes and ports, disabled upstream Sparkle feed, optional Firebase config, and unchanged `lorca` runtime identifiers. Also resolve functional conflicts in `packages/beans-blobatar`, the CLI, relay, agent/tools, and Mac, phone, and Windows/Linux app surfaces: retain encrypted policy reconciliation, conditional roster writes and protocol 3 compatibility, local websocket Host/Origin checks, bot controls and approval flow, avatar rendering, and custom-model refresh. Check actual Runner NIC ACL attachment and allow/deny evidence rather than assuming script staging describes live state.

Before shipping, run CI checks and platform builds/tests for affected Rust, Mac, phone, and desktop paths; exercise app flows for pairing, Pause/capabilities, draft approval, avatars, and provider refresh on actual clients. Roll out relay replicas before protocol-3 clients, verify health and old-client `426` behavior against the intended environment, then verify live client sync and enforcement before claiming deployment or security protection. Never publish a private relay address, signing material, provider keys, or client credentials in merge notes.

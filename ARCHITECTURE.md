# Beans Architecture

Source of truth for how Beans is built. Read this before writing code, then the docs in [Subjects](#subjects) for the parts you change.

Beans is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The Rust CLI runs on macOS, Linux, and Windows. The macOS app, and the app for Windows and Linux, are UIs for the local CLI, which each bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `beans serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM) on macOS: system materials, SF Symbols, Auto Layout, keyboard, accessibility. On Windows and Linux it is a MyGo app, Go and the system webview, that follows the macOS app screen for screen.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials belong to the account.** API keys, subscription tokens and custom-provider connections sync to every paired Device as a `credentials` blob encrypted with the account DEK. A bot uses them on its assigned Runner. Production Grok OAuth is disabled pending a verified Beans contract; the other credential mechanisms remain unchanged.
7. **A bot runs on one Runner:** that Device’s CLI.

## Three processes

```
┌─────────────────────┐     local websocket      ┌──────────────────────────┐
│  Beans.app        │ ◄──────────────────────► │  beans CLI (Rust)      │
│  AppKit             │     127.0.0.1           │  keys + agent loop       │
└─────────────────────┘                          └────────────┬─────────────┘
                                                              │ HTTPS
                                                              │ ciphertext + signed requests
                                                              ▼
                                                 ┌──────────────────────────┐
                                                 │  Relay (Rust, axum +     │
                                                 │  SQLite or Postgres)     │
                                                 │  public keys + blobs     │
                                                 └──────────────────────────┘
```

- **App:** native chat UI. Create or restore identity, pair, through the CLI. Launches the bundled CLI as a child process and restarts it if it exits.
- **CLI:** identity and machine keys, local websocket, encrypt/decrypt, agent loop, the account’s provider credentials, sync with the relay.
- **Relay:** store-and-forward API. Rust, axum, SQLite or Postgres (`crates/relay`). Self-host it anywhere; clients point `BEANS_RELAY_URL` at it.

Beans and Beans Dev on macOS, iOS, and Android retain distribution ids `ai.amoena.beans` and `ai.amoena.beans.dev` and keychain groups `group.ai.amoena.beans` and `group.ai.amoena.beans.dev`. Windows and Linux use Beans package names. Desktop CLI homes are isolated `~/.beans-v2` (port `4874`) and `~/.beans-dev-v2` (port `4875`); phone cores use `beans-v2/core` and `beans-dev-v2/core` inside their separate OS sandboxes.

If the CLI is down, the app shows a native empty state with the launcher’s status and the manual `beans serve --home … --port …` command for that build.

Fresh Beans accounts use format `beans-v2`, HKDF salt `beans-v2`, version-prefixed backups and mandatory protocol 5 with `Beans-Protocol: 5` and `Beans-Format: beans-v2`. Storage validates `format.json` before mutation. Old accounts, unversioned backup phrases and mixed-format pairing/sync are incompatible; no automatic migration or reset occurs. See [Identity](docs/architecture/identity.md) and [Protocols](docs/architecture/protocols.md). Production Grok OAuth is unavailable pending a verified Beans provider contract; other provider and encrypted credential mechanisms remain unchanged.


## Domain model

[Account model](docs/architecture/account-model.md) records every entity, what stays plaintext on Devices, what the relay stores, and how jobs reach the assigned Runner.

## Beans releases and server updates

Stable `beans-v<root version>` releases carry signed schema-1 readiness. Manual dispatch selects immutable `server` or `all` scope before building. Each updater requires its own inventory; server readiness does not authorize clients. All scope coordinates desktop installers and exact-source EAS APK/AAB/store IPA builds. Store submission is separate and manual. See [Releases](docs/architecture/releases.md).

The Linux server updater verifies readiness, upgrades and checks the relay first, then drains and replaces Runners using renewable admission leases. Automation requires trusted bootstrap, root-owned target configuration, configured Runner update tokens and explicit timer enablement after a successful manual pass. These mechanisms do not establish publication or live rollout. See [Runner update drain](docs/architecture/service-updates.md#runner-update-drain).

## Subjects

One doc per subject under `docs/architecture/`, each short enough to read in one pass. `bun run check:docs` holds this file to 16 KiB and each subject to 24 KiB, and checks that every subject is listed here and that links and their anchors resolve.

| Doc | Read it for |
| --- | --- |
| [Account model](docs/architecture/account-model.md) | Entity relationships, Device/relay data ownership and cross-Runner job routing |
| [Identity](docs/architecture/identity.md) | Key pairs and the identity device, pairing and unpairing, Devices and Runners, what the relay sees, the account's provider credentials |
| [Relay](docs/architecture/relay.md) | `crates/relay`: storage on SQLite or Postgres, files, housekeeping, quotas, metrics, rate limits, auth, tables and migrations, the blob, sync socket, and push APIs, deploys |
| [Protocols](docs/architecture/protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [Local API](docs/architecture/local-api.md) | The `/ws` socket: trust boundary, field errors, every method with params and results, events, secret-bearing methods, memory and setup bodies |
| [CLI (runtime)](docs/architecture/runtime.md) | Data directory, local websocket access, proxy/certificate trust, agent loop and notifications |
| [CLI service and updates](docs/architecture/service-updates.md) | CLI installation, standalone services, Runner admission drain and update availability |
| [Tools](docs/architecture/tools.md) | Team, memory, and coding tools, Auto-review |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | A bot's commands in terminals of their own: when a call returns, background commands, the command's card, answering and stopping, Running tasks |
| [Codemode and Plugins](docs/architecture/plugins.md) | Scripts that call plugin tools, MCP plugins and their installs, sign-in, plugin calls at turn time |
| [Marketplace](docs/architecture/marketplace.md) | The relay-backed index of plugins and bot templates, offline fallback and updates, bots added from a template, the marketplace sheet |
| [MCP servers](docs/architecture/mcp-servers.md) | The user's own MCP servers in a Runner's `mcp.json`: the file and other apps' spellings, sign-in, the `mcp.*` methods and `beans mcp`, the apps' MCP Servers section and server sheet |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | The lead bot, DMs and groups, group descriptions and ownership, who answers, handoffs between bots, routines and their checks, a bot's memory |
| [Memory service UI](docs/architecture/memory-ui.md) | Client drafts, masked replies, secret/options patches, consent and capability gates |
| [Memory services](docs/architecture/memory-services.md) | Encrypted config, dispatch, queues, deletion and embeddings |
| [Bot avatars](docs/architecture/avatars.md) | Generated appearance contract, independent photos, validated API edits, encrypted persistence and fresh-format protocol compatibility |
| [Providers](docs/architecture/providers.md) | Each model provider and its sign-in, custom providers, thinking levels, the model catalog and cost, compaction, retries |
| [macOS app](docs/architecture/macos-app.md) | AppKit launch, windows, onboarding, settings, updates, inspector, Blobatar |
| [macOS sidebar](docs/architecture/macos-sidebar.md) | Sidebars, search, native chrome, toolbar navigation |
| [macOS chat](docs/architecture/macos-chat.md) | AppKit transcript, composer, attachments, dictation, and working state |
| [Windows and Linux app](docs/architecture/desktop-app.md) | The MyGo app: its Go side and Solid page, title bar, commands, updates, development and builds |
| [Phone app](docs/architecture/phone-app.md) | The Expo app over the Rust core: the native module, pairing, relay status, attachments, dictation, notifications, turns |
| [Phone interface](docs/architecture/phone-interface.md) | Native screens, transcript presentation and adaptive navigation |
| [Releases](docs/architecture/releases.md) | Signed readiness, desktop/EAS builds, testing submissions, accounts and credentials |
| [Website](docs/architecture/website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese in each app, and what the CLI words |

## Repo layout

```
beans/
  ARCHITECTURE.md      # this overview and the list of subjects
  README.md
  docs/architecture/   # one doc per subject
  docs/agent/          # beans-agent's own documentation
  Cargo.toml           # workspace
  crates/agent/        # beans-agent: loop, tools, codemode (QuickJS), and Messages, Chat Completions, Responses, ChatGPT, and Grok providers
  crates/models/       # beans-models: the bundled model catalog (windows, thinking levels, rates); Devices check the selected Beans relay's public feed for updates
  crates/provider-auth/ # OAuth token types and PKCE flows shared by every Device
  crates/tls/          # beans-tls: the certificate trust of every Device's HTTPS, the system's on macOS and Windows
  crates/cli/          # beans: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # beans-mobile: the core for the phone over UniFFI
  crates/markdown/     # beans-markdown: message Markdown as the blocks and spans every app renders (pulldown-cmark, and GitHub's autolinks for bare URLs and addresses), for the Mac and phone over UniFFI
  crates/relay/        # beans-relay: axum + SQLite or Postgres, and its Dockerfile
  macos/               # AppKit SPM app; the build bundles the CLI
  desktop/             # the Windows and Linux app: MyGo (Go + system webview) with a Solid page; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/beans-core)
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build, macOS release, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release.yml: manual signed Beans releases; test.yml: full release-PR test matrix; docs.yml: release-PR doc check; catalog.yml: release-PR catalog/index check
```

`bun run android` rebuilds the Rust core for Android, then builds and runs the Expo dev client on the Android emulator. `cd mobile && bun run core` rebuilds the Rust core for both phone platforms; `bun run mobile:dev` is the iOS development loop described below.

`bun run dev` runs the macOS Beans Dev loop; `bun run build` produces the Beans release bundle. See [macOS development and builds](docs/architecture/macos-app.md#development-and-builds) for rebuild, launch and SDK-stamping mechanisms. `bun run relay` runs a local relay. The [phone development loop](docs/architecture/phone-app.md#development-loop) fingerprints native inputs, rebuilds stale pieces and leaves production Beans processes alone.

## Status

Implemented mechanisms include crypto and blob sync, relay storage, CLI identity/pairing/restore, the local websocket, API-key and ChatGPT subscription providers, server-side web search, the agent loop, group chats, cross-Runner jobs and handoffs, steering/stop, routines, MCP plugins, permission cards, encrypted phone pushes, `beans service`, and signed app/server update readiness with Runner admission drain. Standalone CLI self-update is unavailable; production Grok OAuth is disabled pending a verified Beans contract. Source implementation is not installed-app or live-service acceptance.

Next: keychain storage, a cost budget per chat.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The Device that creates or restores the identity holds the master secret.

## Open points

- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.

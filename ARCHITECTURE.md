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

Signed release scopes and client inventories are defined in [Releases](docs/architecture/releases.md). The [server updater](docs/architecture/service-updates.md#runner-update-drain) verifies readiness, updates the relay first and drains Runners before replacement.

## Subjects

One doc per subject under `docs/architecture/`, each short enough to read in one pass. `bun run check:docs` holds this file to 16 KiB and each subject to 24 KiB, and checks that every subject is listed here and that links and their anchors resolve.

| Doc | Read it for |
| --- | --- |
| [Account model](docs/architecture/account-model.md) | Entities, Device/relay ownership and cross-Runner routing |
| [Identity](docs/architecture/identity.md) | Keys, pairing, Devices/Runners and encrypted credentials |
| [Relay](docs/architecture/relay.md) | Storage backends, auth, quotas, sync, files and push APIs |
| [Protocols](docs/architecture/protocols.md) | App/CLI websocket and encrypted CLI/relay messages |
| [Managed policy receipts and replay](docs/architecture/managed-policy-replay.md) | Opaque policy commits, receipts, replay cuts and teardown |
| [Local API](docs/architecture/local-api.md) | Methods, events, validation and secret-bearing responses |
| [CLI (runtime)](docs/architecture/runtime.md) | Commands, transport, agent loop and diagnostics |
| [Local storage and startup admission](docs/architecture/storage-admission.md) | Data directories, format checks and startup persistence |
| [CLI service and updates](docs/architecture/service-updates.md) | Services, admission drain and update availability |
| [Tools](docs/architecture/tools.md) | Team, memory and coding tools; Auto-review |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | Commands, background sessions, input and cancellation |
| [Codemode and Plugins](docs/architecture/plugins.md) | Scripts, MCP installs, sign-in and tool calls |
| [Skills](docs/architecture/skills.md) | Discovery, managed copies, provenance and removal |
| [Marketplace](docs/architecture/marketplace.md) | Plugin/bot index, offline fallback, templates and updates |
| [MCP servers](docs/architecture/mcp-servers.md) | Runner configuration, sign-in, API and client settings |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | Chats, handoffs, routines and bot memory |
| [Memory service UI](docs/architecture/memory-ui.md) | Drafts, masked replies, consent and capabilities |
| [Memory services](docs/architecture/memory-services.md) | Encrypted config, queues, deletion and embeddings |
| [Bot avatars](docs/architecture/avatars.md) | Appearance validation, photos and encrypted persistence |
| [Providers](docs/architecture/providers.md) | Providers, custom connections, thinking and retries |
| [Model catalog and cost](docs/architecture/model-catalog.md) | Model metadata, pricing, usage and refresh |
| [macOS app](docs/architecture/macos-app.md) | AppKit lifecycle, onboarding, settings and updates |
| [macOS sidebar](docs/architecture/macos-sidebar.md) | Navigation, search and native chrome |
| [macOS chat](docs/architecture/macos-chat.md) | Transcript, composer, attachments and dictation |
| [Windows and Linux app](docs/architecture/desktop-app.md) | MyGo host, Solid page, commands, updates and builds |
| [Phone app](docs/architecture/phone-app.md) | Expo/Rust core, pairing, sync, notifications and turns |
| [Phone releases and updates](docs/architecture/phone-updates.md) | TestFlight, Android releases and signed app updates |
| [Phone interface](docs/architecture/phone-interface.md) | Native screens, transcript and adaptive navigation |
| [Releases](docs/architecture/releases.md) | Signed readiness, desktop/EAS builds and submission |
| [Website](docs/architecture/website.md) | Website, documentation and install scripts |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese across clients |
| [Portable setup](docs/architecture/portable-setup.md) | Secret-free configuration recipe boundaries, versioning, ownership, validation and import transaction contract |

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
  .github/workflows/   # release.yml: manual signed Beans releases; test.yml: full PR/manual test matrix; docs.yml: PR/manual doc check; catalog.yml: PR/manual catalog/index check
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

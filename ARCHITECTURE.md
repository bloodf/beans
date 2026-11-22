# Beans Architecture

Before coding, read this and [Subjects](#subjects).

Beans is a Grok Bot alternative with persistent named bots, 1:1 and group chats, handoff and orchestration on **Devices you own**. Its Rust CLI runs on macOS, Linux and Windows; each desktop UI bundles and launches it.

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

Fresh Beans accounts use format `beans-v2`, HKDF salt `beans-v2`, version-prefixed backups and mandatory protocol 5 with `Beans-Protocol: 5` and `Beans-Format: beans-v2`. Storage validates `format.json` before mutation. Old accounts, unversioned backup phrases and mixed-format pairing/sync are incompatible; no automatic migration or reset occurs. See [Identity](docs/architecture/identity.md) and [Protocols](docs/architecture/protocols.md).

## Domain model

[Account model](docs/architecture/account-model.md) records every entity, what stays plaintext on Devices, what the relay stores, and how jobs reach the assigned Runner.

## Beans releases and server updates

Stable `beans-v<root version>` releases carry signed schema-1 readiness. Manual dispatch selects immutable `server` or `all` scope before building. Each updater requires its own inventory; server readiness does not authorize clients. All scope coordinates desktop installers and exact-source EAS APK/AAB/store IPA builds. Store submission is separate and manual. See [Releases](docs/architecture/releases.md).

The Linux server updater verifies readiness, upgrades and checks the relay first, then drains and replaces Runners using renewable admission leases. Automation requires trusted bootstrap, root-owned target configuration, configured Runner update tokens and explicit timer enablement after a successful manual pass. These mechanisms do not establish publication or live rollout. See [Runner update drain](docs/architecture/service-updates.md#runner-update-drain).

## Subjects

Each subject has one doc in `docs/architecture/`. `pnpm run check:docs` enforces the 16 KiB overview and 24 KiB subject limits, complete subject listings and valid links/anchors.

| Doc | Read it for |
| --- | --- |
| [Account model](docs/architecture/account-model.md) | Entities, Device/relay data ownership and cross-Runner routing |
| [Identity](docs/architecture/identity.md) | Key pairs, pairing and unpairing, Devices and Runners, what the relay sees, account credentials, [keystore](docs/architecture/keystore.md) |
| [Device names](docs/architecture/device-names.md) | Account aliases, machine names and durable roster intent |
| [Device revocation](docs/architecture/device-revocation.md) | Lost-Device boundaries; proposed containment and rotation |
| [Relay](docs/architecture/relay.md) | `crates/relay`: SQLite/Postgres storage, files, quotas, auth, blob/sync/push APIs, deploys |
| [Computer display](docs/architecture/computer-display.md) | Proposed authenticated display/input transport and its gaps |
| [Protocols](docs/architecture/protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [Computer ownership](docs/architecture/computer-ownership.md) | Proposed encrypted ownership, lifecycle and old-client fixture |
| [Computer ownership compatibility](docs/architecture/computer-ownership-compatibility.md) | Source-extracted fixture, hashes, replay and limits |
| [Managed policy receipts and replay](docs/architecture/managed-policy-replay.md) | Opaque policy commits, receipts, replay cuts and teardown |
| [Local API](docs/architecture/local-api.md) | The `/ws` socket: trust boundary, every method, events, secret-bearing methods |
| [CLI (runtime)](docs/architecture/runtime.md) | Commands, local websocket access, proxy trust, diagnostics, agent loop |
| [Diagnostics report](docs/architecture/diagnostics.md) | Privacy-safe setup report and injected computer-host checks |
| [Local storage and startup admission](docs/architecture/storage-admission.md) | Data directory, format admission, staged preload, startup persistence |
| [Local task authority](docs/architecture/tasks.md) | Execution ownership, Job.id admission, receipts, recovery, replay fences |
| [CLI service and updates](docs/architecture/service-updates.md) | Installation, standalone services, admission drain, update availability |
| [Tools](docs/architecture/tools.md) | Team, memory, and coding tools, Auto-review, [secrets](docs/architecture/secrets.md) |
| [Computer provenance](docs/architecture/computer-provenance.md) | Proposed computer-stack provenance and verification gates |
| [Computer threat model](docs/architecture/computer-threat-model.md) | Planned computer boundaries, fail-closed requirements, residual risks |
| [Camofox evaluation](docs/architecture/camofox.md) | Pinned Camofox source evaluation and its open no-go gates |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | Command terminals: returns, background commands, cards, input, stopping |
| [Realtime voice evaluation](docs/architecture/voice-evaluation.md) | Voice stack options, transports, privacy and pending evidence |
| [Codemode and Plugins](docs/architecture/plugins.md) | Plugin-calling scripts, MCP plugins, installs, sign-in, turn-time calls |
| [Skills](docs/architecture/skills.md) | Discovery, managed copies, provenance, mounted checks and uninstall |
| [Marketplace](docs/architecture/marketplace.md) | Relay-backed plugin and bot-template index, offline fallback, updates |
| [MCP servers](docs/architecture/mcp-servers.md) | A Runner's `mcp.json`, sign-in, `mcp.*` methods, `beans mcp`, app settings |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | Lead bot, DMs and groups, who answers, handoffs, routines, bot memory |
| [Memory service UI](docs/architecture/memory-ui.md) | Client drafts, masked replies, patches, consent and capability gates |
| [Memory services](docs/architecture/memory-services.md) | Encrypted config, dispatch, queues, deletion and embeddings |
| [Schedule models](docs/architecture/schedule-models.md) | Local CPU models: catalog, installs, routine detection, [semantic search](docs/architecture/semantic-search.md) |
| [Bot avatars](docs/architecture/avatars.md) | Appearance contract, photos, validated edits and encrypted persistence |
| [Providers](docs/architecture/providers.md) | Each model provider and its sign-in, custom providers, thinking levels, compaction, retries, [typed decisions](docs/architecture/typed-decisions.md) |
| [Zhipu GLM compatibility](docs/architecture/glm-compatibility.md) | Chosen contract, vendor docs, reasoning-tool continuation |
| [Model catalog and cost](docs/architecture/model-catalog.md) | Model metadata, pricing completeness, usage and catalog refresh |
| [macOS app](docs/architecture/macos-app.md) | AppKit launch, windows, onboarding, settings, updates, inspector, Blobatar |
| [macOS sidebar](docs/architecture/macos-sidebar.md) | Sidebars, search, native chrome, toolbar navigation |
| [macOS chat](docs/architecture/macos-chat.md) | AppKit transcript, composer, attachments, dictation, working state |
| [Composer capabilities](docs/architecture/composer-capabilities.md) | Proposed context/action consumers and their boundaries |
| [Composer](docs/architecture/composer.md) | Client inventory; action/reference syntax, authority and compatibility |
| [Windows and Linux app](docs/architecture/desktop-app.md) | The MyGo app: Go side, Solid page, commands, updates, builds |
| [Phone app](docs/architecture/phone-app.md) | Expo over the Rust core: native module, pairing, attachments, notifications, turns |
| [Phone releases and updates](docs/architecture/phone-updates.md) | TestFlight, Android APKs and the Android self-update flow |
| [Phone interface](docs/architecture/phone-interface.md) | Native screens, transcript presentation and adaptive navigation |
| [Releases](docs/architecture/releases.md) | Signed readiness, desktop/EAS builds, submissions, credentials, [tests](docs/architecture/testing.md) |
| [Website](docs/architecture/website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese in each app, and what the CLI words |
| [Portable setup](docs/architecture/portable-setup.md) | Secret-free setup recipes: versioning, ownership, validation, import |

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
  scripts/             # Node/TypeScript scripts: dev loop, bundle build, macOS release, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release.yml: manual signed Beans releases; test.yml: manual test matrix; docs.yml: manual doc check; catalog.yml: manual catalog/index check
```

See [Phone development](docs/architecture/phone-app.md#development-loop) for phone rebuild commands.

`pnpm run dev` runs the macOS Beans Dev loop; `pnpm run build` produces the Beans release bundle. See [macOS development and builds](docs/architecture/macos-app.md#development-and-builds) for rebuild, launch and SDK-stamping mechanisms. `pnpm run relay` runs a local relay. The [phone development loop](docs/architecture/phone-app.md#development-loop) fingerprints native inputs, rebuilds stale pieces and leaves production Beans processes alone.

## Status

Implemented mechanisms include crypto and blob sync, relay storage, CLI identity/pairing/restore, the local websocket, API-key and ChatGPT subscription providers, server-side web search, the agent loop, group chats, cross-Runner jobs and handoffs, steering/stop, routines, MCP plugins, permission cards, encrypted phone pushes, `beans service`, and signed app/server update readiness with Runner admission drain. Standalone CLI self-update is unavailable; production Grok OAuth is disabled pending a verified Beans contract. Source implementation is not installed-app or live-service acceptance.

Next: keychain storage, a cost budget per chat.

See [Phone app](docs/architecture/phone-app.md) for phone identity and key ownership.

## Open points

- Native keychain backend selection and qualification for master secrets and credentials. The inert [keystore contract](docs/architecture/keystore.md) provides verified copying with fake tests; production retains private files.

When those are chosen, update this file.

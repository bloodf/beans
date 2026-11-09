# Beans Architecture

Before coding, read this and [Subjects](#subjects).

Beans is a Grok Bot alternative with persistent named bots, 1:1 and group chats, handoff and orchestration on **Devices you own**. Its Rust CLI runs on macOS, Linux and Windows; each desktop UI bundles and launches it.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `beans serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM) on macOS: system materials, SF Symbols, Auto Layout, keyboard, accessibility. On Windows and Linux it is a MyGo app, Go and the system webview, with a Solid default interface and an opt-in native Go interface.
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
| [Account model](docs/architecture/account-model.md) | Reference |
| [Identity](docs/architecture/identity.md) | Reference |
| [Device names](docs/architecture/device-names.md) | Reference |
| [Device revocation](docs/architecture/device-revocation.md) | Reference |
| [Relay](docs/architecture/relay.md) | Reference |
| [Computer display](docs/architecture/computer-display.md) | Reference |
| [Protocols](docs/architecture/protocols.md) | Reference |
| [Computer ownership](docs/architecture/computer-ownership.md) | Reference |
| [Computer ownership compatibility](docs/architecture/computer-ownership-compatibility.md) | Reference |
| [Managed policy receipts and replay](docs/architecture/managed-policy-replay.md) | Reference |
| [Local API](docs/architecture/local-api.md) | Reference |
| [CLI (runtime)](docs/architecture/runtime.md) | Reference |
| [Local storage and startup admission](docs/architecture/storage-admission.md) | Reference |
| [Local task authority](docs/architecture/tasks.md) | Reference |
| [CLI service and updates](docs/architecture/service-updates.md) | Reference |
| [Tools](docs/architecture/tools.md) | Reference |
| [Computer provenance](docs/architecture/computer-provenance.md) | Reference |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | Reference |
| [Realtime voice evaluation](docs/architecture/voice-evaluation.md) | Reference |
| [Codemode and Plugins](docs/architecture/plugins.md) | Reference |
| [Skills](docs/architecture/skills.md) | Reference |
| [Marketplace](docs/architecture/marketplace.md) | Reference |
| [MCP servers](docs/architecture/mcp-servers.md) | Reference |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | Reference |
| [Memory service UI](docs/architecture/memory-ui.md) | Reference |
| [Memory services](docs/architecture/memory-services.md) | Reference |
| [Bot avatars](docs/architecture/avatars.md) | Reference |
| [Providers](docs/architecture/providers.md) | Reference |
| [Zhipu GLM compatibility](docs/architecture/glm-compatibility.md) | Reference |
| [Model catalog and cost](docs/architecture/model-catalog.md) | Reference |
| [macOS app](docs/architecture/macos-app.md) | Reference |
| [macOS sidebar](docs/architecture/macos-sidebar.md) | Reference |
| [macOS chat](docs/architecture/macos-chat.md) | Reference |
| [Composer capabilities](docs/architecture/composer-capabilities.md) | Reference |
| [Composer](docs/architecture/composer.md) | Reference |
| [Windows and Linux app](docs/architecture/desktop-app.md) | Reference |
| [Phone app](docs/architecture/phone-app.md) | Reference |
| [Phone releases and updates](docs/architecture/phone-updates.md) | Reference |
| [Phone interface](docs/architecture/phone-interface.md) | Reference |
| [Releases](docs/architecture/releases.md) | Reference |
| [Website](docs/architecture/website.md) | Reference |
| [Languages](docs/architecture/languages.md) | Reference |
| [Portable setup](docs/architecture/portable-setup.md) | Reference |

| [Relay trust](docs/architecture/relay-trust.md) | Reference |
| [Requests between Devices](docs/architecture/requests.md) | Reference |
| [Notifications](docs/architecture/notifications.md) | Reference |
| [Bot permissions](docs/architecture/bot-permissions.md) | Reference |
| [Auto-review](docs/architecture/auto-review.md) | Reference |
| [Handoffs](docs/architecture/handoffs.md) | Reference |
| [Review queue](docs/architecture/review-queue.md) | Reference |
| [Message drafts](docs/architecture/drafts.md) | Reference |
| [Outputs and evidence](docs/architecture/outputs.md) | Reference |
| [Stop](docs/architecture/stopping.md) | Reference |
| [Coding agents](docs/architecture/coding-agents.md) | Reference |
| [Coding agent roster](docs/architecture/agent-roster.md) | Reference |
| [Secret requests](docs/architecture/secrets.md) | Reference |
| [Browser sessions](docs/architecture/browser-sessions.md) | Reference |
| [Integrations and named accounts](docs/architecture/integrations.md) | Reference |
| [Workflow onboarding](docs/architecture/workflows.md) | Reference |
| [Bot templates](docs/architecture/templates.md) | Reference |
| [Routines](docs/architecture/routines.md) | Reference |
| [One-time routines, routines on events, and calendar events](docs/architecture/routine-triggers.md) | Reference |
| [Event triggers](docs/architecture/event-triggers.md) | Reference |
| [Hosted receivers](docs/architecture/receivers.md) | Reference |
| [Channels](docs/architecture/channels.md) | Reference |
| [Channel readers](docs/architecture/channel-readers.md) | Reference |
| [Coordinator attention](docs/architecture/attention.md) | Reference |
| [Shared project context](docs/architecture/project-context.md) | Reference |
| [Playbooks](docs/architecture/playbooks.md) | Reference |
| [Workflow feedback](docs/architecture/feedback.md) | Reference |
| [Compaction and usage](docs/architecture/compaction.md) | Reference |
| [Budgets and connector limits](docs/architecture/budgets.md) | Reference |
| [Sidebar sections and alerts](docs/architecture/sidebar.md) | Reference |
| [Desktop app updates](docs/architecture/desktop-updates.md) | Reference |
| [Phone layout](docs/architecture/phone-layout.md) | Reference |
| [Repo layout and development](docs/architecture/development.md) | Reference |
| [Status](docs/architecture/status.md) | Reference |

| [Account lifecycle](docs/architecture/account-lifecycle.md) | Reference |
| [Agent runtime](docs/architecture/agent-runtime.md) | Reference |
| [Bot automation](docs/architecture/bot-automation.md) | Reference |
| [Bot memory and feedback](docs/architecture/bot-memory.md) | Reference |
| [Durable tasks](docs/architecture/durable-tasks.md) | Reference |
| [Approval and policy history](docs/architecture/policy-history.md) | Reference |
| [Relay protocol](docs/architecture/relay-protocol.md) | Reference |
| [Subject guide](docs/architecture/subject-guide.md) | Reference |

| [Implementation status](docs/architecture/implementation-status.md) | Implementation boundaries |
| [Repository layout](docs/architecture/repository-layout.md) | Crates and development loops |

## Repo layout

See [Repository layout](docs/architecture/repository-layout.md).

## Status

See [Implementation status](docs/architecture/implementation-status.md). Source mechanisms and client coverage are distinct. AppKit upstream screen candidates cover Attention, Tasks and Handoffs only; other upstream screens and desktop native lifecycle integration remain incomplete. Website download routes call `latestMacRelease`; appcast failure disables only Mac. Website readiness-asset selection does not establish cryptographic verification or runtime acceptance.

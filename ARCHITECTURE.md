# Lorca Architecture

Source of truth for how Lorca is built. Read this before writing code, then the docs in [Subjects](#subjects) for the parts you change.

Lorca is a Grok Bot alternative: persistent named bots, 1:1 chats, group chats, handoff, and orchestration. Work runs on **Devices you own**. The Rust CLI runs on macOS, Linux, and Windows. The macOS app, and the app for Windows and Linux, are UIs for the local CLI, which each bundles and launches.

Identity is a **key pair**. Devices pair. The relay stores public keys and ciphertext, after [Happy’s security model](https://happy.engineering/docs/security/).

## Constraints

1. **The app speaks only to the local CLI** over localhost websocket. The app ships the CLI binary inside its bundle and starts `lorca serve` itself, unless one already answers on the port. The CLI holds keys, talks to the relay, and talks to models.
2. **The UI is AppKit** (SPM) on macOS: system materials, SF Symbols, Auto Layout, keyboard, accessibility. On Windows and Linux it is a MyGo app, Go and the system webview, that follows the macOS app screen for screen.
3. **The CLI owns the agent loop:** inference, tools, streaming, cancellation, orchestration.
4. **The relay is zero-knowledge:** opaque blobs and public keys. Auth is a signature challenge.
5. **Every Device records its `os`.** A Device with a desktop `os` (`macos`, `linux`, `windows`) is a **Runner**. Phones and tablets (`ios`, `ipados`, `android`) are Devices, never Runners.
6. **Provider credentials belong to the account.** API keys, ChatGPT and Grok tokens, and custom providers (any server that speaks OpenAI's or Anthropic's API) are connected once, on any Device, and reach every paired Device as a `credentials` blob encrypted with the account DEK. A bot runs with them on whichever Runner it is assigned to.
7. **A bot runs on one Runner:** that Device’s CLI.

## Three processes

```
┌─────────────────────┐     local websocket      ┌──────────────────────────┐
│  Lorca.app        │ ◄──────────────────────► │  lorca CLI (Rust)      │
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
- **Relay:** store-and-forward API. Rust, axum, SQLite or Postgres (`crates/relay`). Self-host it anywhere; clients point `LORCA_RELAY_URL` at it.

Beans and Beans Dev installs on macOS, iOS, and Android use `ai.amoena.beans` and `ai.amoena.beans.dev`; the Windows and Linux app keeps its upstream identity. The macOS Beans app runs its CLI with `--home ~/.beans --port 4864`, and Beans Dev uses `--home ~/.beans-dev --port 4865`. On phones the distinct application ids give each build its own OS sandbox, and the core uses `lorca/core` or `lorca-dev/core` inside that sandbox. Their iOS keychain groups are `group.ai.amoena.beans` and `group.ai.amoena.beans.dev`.

If the CLI is down, the app shows a native empty state with the launcher’s status and the manual `lorca serve --home … --port …` command for that build.

## Domain model

Plaintext lives **on Devices**:

```
Identity 1──* Device
Device   1──* Bot          (only a Runner: os is macos, linux, or windows)
Identity 1──* Chat
Chat     *──* Bot          (kind dm: exactly 1 bot, fixed · kind group: 1–6 bots, members change)
Chat     1──* Message
Bot      1──* Routine      (a scheduled task, run in the bot's DM on its Runner)
Device   1──* Plugin       (an MCP server installed on a Runner or in its mcp.json, for every bot there)
Bot      1──* Job          (a turn on the bot's Runner)
```

| Entity             | Device                                                    | Relay                                                  |
| ------------------ | --------------------------------------------------------- | ------------------------------------------------------ |
| Identity           | Master + content + signing keys                           | Public key                                             |
| Device             | Machine keypair, `os`                                     | Machine public key + encrypted metadata blob           |
| Bot                | Decrypted profile                                         | Inside encrypted roster blobs                          |
| Routine            | Name, schedule, prompt, state                             | Inside encrypted roster blobs                          |
| Plugin             | Manifest, variables, secrets, tokens on the Runner        | Id and state inside the Runner's encrypted machine blob |
| ProviderCredential | `credentials.json` on every Device                        | Inside the encrypted `credentials` blob                |
| Chat / Message     | Account/chat DEK                                          | Encrypted blobs                                        |
| Job                | Any paired Device may create; the assigned Runner runs it | Sealed envelope to that Runner’s machine box key; deleted once run. A hard Stop sends `job_cancel` to the Device running it; the Runner seals how the turn ended (`job_result`) to the requesting Device, and lists the turn and what it is doing in its `machine` blob for every Device |

A bot's deterministic Blobatar is seeded by its stable `bot.id`. The core stores optional generated `look` settings in the encrypted roster, independently of the photo attachment in `avatar`; appearance-preserving roster writers require protocol 4. See [Bot avatars](docs/architecture/avatars.md) for the shared contract, API, photos, persistence, and rollout requirements.

Creating a bot for Runner B from Device A: A writes an encrypted bot profile into the roster (paired Devices can read it) and pins B’s machine id. Bot create rejects a target whose `os` is not desktop. Turns are job envelopes addressed to B. B decrypts the job, runs the loop with the account’s provider credentials, and uploads encrypted replies.

If B is offline, the envelope waits on the relay until B fetches it. The UI infers that from decrypted roster state. A turn for a provider the account has not connected ends with a notice in the chat that says to connect it in Settings.

## Beans releases and server updates

Stable `beans-v<root version>` releases carry signed schema-1 readiness. Manual dispatch selects immutable `server` or `all` scope before building. Each updater requires its own inventory; server readiness does not authorize clients. All scope coordinates desktop installers and exact-source EAS APK/AAB/store IPA builds. Store submission is separate and manual. See [Releases](docs/architecture/releases.md).

The Linux server updater verifies readiness, upgrades and checks the relay first, then drains and replaces Runners using renewable admission leases. Automation requires trusted bootstrap, root-owned target configuration, configured Runner update tokens and explicit timer enablement after a successful manual pass. These mechanisms do not establish publication or live rollout. See [Runner update drain](docs/architecture/runtime.md#runner-update-drain).

## Subjects

One doc per subject under `docs/architecture/`, each short enough to read in one pass. `bun run check:docs` holds this file to 16 KiB and each subject to 24 KiB, and checks that every subject is listed here and that links and their anchors resolve.

| Doc | Read it for |
| --- | --- |
| [Identity](docs/architecture/identity.md) | Key pairs and the identity device, pairing and unpairing, Devices and Runners, what the relay sees, the account's provider credentials |
| [Relay](docs/architecture/relay.md) | `crates/relay`: storage on SQLite or Postgres, files, housekeeping, quotas, metrics, rate limits, auth, tables and migrations, the blob, sync socket, and push APIs, deploys |
| [Protocols](docs/architecture/protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [CLI (runtime)](docs/architecture/runtime.md) | The `lorca` binary and its data directory, installing it, local websocket access, system proxies and certificate trust, Runner update drain, the agent loop and a turn on a Runner, notifications |
| [Tools](docs/architecture/tools.md) | Team, memory, and coding tools, Auto-review |
| [Terminal sessions](docs/architecture/terminal-sessions.md) | A bot's commands in terminals of their own: when a call returns, background commands, the command's card, answering and stopping, Running tasks |
| [Codemode and Plugins](docs/architecture/plugins.md) | Scripts that call plugin tools, MCP plugins and their installs, sign-in, plugin calls at turn time |
| [Marketplace](docs/architecture/marketplace.md) | The relay-backed index of plugins and bot templates, offline fallback and updates, bots added from a template, the marketplace sheet |
| [MCP servers](docs/architecture/mcp-servers.md) | The user's own MCP servers in a Runner's `mcp.json`: the file and other apps' spellings, sign-in, the `mcp.*` methods and `lorca mcp`, the apps' MCP Servers section and server sheet |
| [Bots, Routines, and Memory](docs/architecture/bots.md) | The lead bot, DMs and groups, group descriptions and ownership, who answers, handoffs between bots, routines and their checks, a bot's memory |
| [Memory service UI](docs/architecture/memory-ui.md) | Client drafts, masked replies, secret/options patches, consent and capability gates |
| [Memory services](docs/architecture/memory-services.md) | Encrypted config, dispatch, queues, deletion and embeddings |
| [Bot avatars](docs/architecture/avatars.md) | Generated appearance contract, independent photos, validated API edits, encrypted persistence and protocol-4 rollout |
| [Providers](docs/architecture/providers.md) | Each model provider and its sign-in, custom providers, thinking levels, the model catalog and cost, compaction, retries |
| [macOS app](docs/architecture/macos-app.md) | AppKit launch, windows, onboarding, settings, updates, inspector, Blobatar |
| [macOS sidebar](docs/architecture/macos-sidebar.md) | Sidebars, search, native chrome, toolbar navigation |
| [macOS chat](docs/architecture/macos-chat.md) | AppKit transcript, composer, attachments, dictation, and working state |
| [Windows and Linux app](docs/architecture/desktop-app.md) | The MyGo app: its Go side and Solid page, title bar, commands, updates, development and builds |
| [Phone app](docs/architecture/phone-app.md) | The Expo app over the Rust core: the native module, pairing, relay status, attachments, dictation, notifications, turns |
| [Releases](docs/architecture/releases.md) | Signed readiness, desktop/EAS builds, testing submissions, accounts and credentials |
| [Website](docs/architecture/website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](docs/architecture/languages.md) | English and Simplified Chinese in each app, and what the CLI words |

## Repo layout

```
lorca/
  ARCHITECTURE.md      # this overview and the list of subjects
  README.md
  docs/architecture/   # one doc per subject
  docs/agent/          # lorca-agent's own documentation
  Cargo.toml           # workspace
  crates/agent/        # lorca-agent: loop, tools, codemode (QuickJS), and Messages, Chat Completions, Responses, ChatGPT, and Grok providers
  crates/models/       # lorca-models: the bundled model catalog (windows, thinking levels, rates); Devices check the selected Beans relay's public feed for updates
  crates/provider-auth/ # OAuth token types and PKCE flows shared by every Device
  crates/tls/          # lorca-tls: the certificate trust of every Device's HTTPS, the system's on macOS and Windows
  crates/cli/          # lorca: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # lorca-mobile: the core for the phone over UniFFI
  crates/markdown/     # lorca-markdown: message Markdown as the blocks and spans every app renders (pulldown-cmark, and GitHub's autolinks for bare URLs and addresses), for the Mac and phone over UniFFI
  crates/relay/        # lorca-relay: axum + SQLite or Postgres, and its Dockerfile
  macos/               # AppKit SPM app; the build bundles the CLI
  desktop/             # the Windows and Linux app: MyGo (Go + system webview) with a Solid page; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/lorca-core)
  web/                 # the site
  scripts/             # bun scripts: dev loop, bundle build, macOS release, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release.yml: unified signed Beans releases; test.yml: every app's and crate's tests on each pull request; docs.yml: the doc check
```

`bun run android` rebuilds the Rust core for Android, then builds and runs the Expo dev client on the Android emulator. `cd mobile && bun run core` rebuilds the Rust core for both phone platforms; `bun run mobile:dev` is the iOS development loop described below.

`bun run dev` runs the macOS Beans Dev loop; `bun run build` produces the Beans release bundle. See [macOS development and builds](docs/architecture/macos-app.md#development-and-builds) for rebuild, launch and SDK-stamping mechanisms. `bun run relay` runs a local relay. `bun run mobile:dev` (`scripts/mobile.ts`) is the Beans Dev phone loop on the iOS Simulator, or on `--device <name or udid>`: it fingerprints the crates the phone links, the prebuild inputs (Expo config, assets, `package.json`, plugins, targets), the pod inputs with the checkout's path, and the native module sources (stamps in `mobile/.expo/dev-stamps.json`), rebuilds what is stale (`bun run core ios`, a clean `expo prebuild`, `pod install`, `expo run:ios`), starts Metro, and opens the dev client on it. A Rust save while it runs rebuilds the core and installs the app again. The Mac and phone loops leave production Beans processes alone, so all builds run side by side.

## Status

Done: crypto and blob protocol, relay, CLI (identity, pairing, restore, local WS, API-key and subscription providers, server-side web search, agent loop, encrypt-before-upload, group chats, cross-Runner jobs and handoffs, steering and stop, routines, plugins over MCP with a marketplace, the user's own servers in `mcp.json`, and permission cards, encrypted pushes for replies, failures, and pending confirmations), app wiring and the bundled CLI launcher.

Next: keychain storage, a cost budget per chat.

The phone app (`mobile/`) pairs as a Device with `os` `ios`, `ipados`, or `android`; it is never a Runner and does not hold the master secret. The Device that creates or restores the identity holds the master secret.

## Open points

- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.

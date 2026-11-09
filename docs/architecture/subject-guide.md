# Subject guide

## Subjects

Each subject has one doc in ``. `pnpm run check:docs` enforces the 16 KiB overview and 24 KiB subject limits, complete subject listings and valid links/anchors.

| Doc | Read it for |
| --- | --- |
| [Account model](account-model.md) | Entity relationships, Device/relay data ownership and cross-Runner job routing |
| [Identity](identity.md) | Key pairs and the identity device, pairing and unpairing, Devices and Runners, what the relay sees, the account's provider credentials |
| [Device names](device-names.md) | Account aliases, machine names and durable roster intent |
| [Device revocation](device-revocation.md) | Lost, un-wiped Device boundaries and the proposed, unimplemented containment and rotation contract |
| [Relay](relay.md) | `crates/relay`: storage on SQLite or Postgres, files, housekeeping, quotas, metrics, rate limits, auth, tables and migrations, the blob, sync socket, and push APIs, deploys |
| [Computer display](computer-display.md) | Proposed authenticated display/input transport, Device-bound sessions, lifecycle fences, client feasibility and measurement gaps |
| [Protocols](protocols.md) | The app ↔ CLI websocket and the CLI ↔ relay requests and blobs |
| [Computer ownership](computer-ownership.md) | Proposed encrypted ownership, async lifecycle, capability compatibility and old-client fixture |
| [Computer ownership compatibility](computer-ownership-compatibility.md) | Source-extracted fixture, hashes, replay and evidence limits |
| [Managed policy receipts and replay](managed-policy-replay.md) | Staged opaque policy/roster commits, durable receipt retention, authenticated finite replay cuts and teardown boundaries |
| [Local API](local-api.md) | The `/ws` socket: trust boundary, field errors, every method with params and results, events, secret-bearing methods, memory and setup bodies |
| [CLI (runtime)](runtime.md) | Commands, local websocket access, proxy/certificate trust, diagnostics, agent loop and notifications |
| [Local storage and startup admission](storage-admission.md) | Data directory, format admission, staged scratch preload, creation guards and final startup persistence |
| [Local task authority](tasks.md) | Exclusive execution ownership, durable Job.id admission, invocation receipts, recovery and replay fences |
| [CLI service and updates](service-updates.md) | CLI installation, standalone services, Runner admission drain and update availability |
| [Tools](tools.md) | Team, memory, and coding tools, Auto-review |
| [Computer provenance](computer-provenance.md) | Proposed computer-stack discovery contract, dependency/license provenance, artifact verification and pending deployment gates |
| [Terminal sessions](terminal-sessions.md) | A bot's commands in terminals of their own: when a call returns, background commands, the command's card, answering and stopping, Running tasks |
| [Realtime voice evaluation](voice-evaluation.md) | Pinned voice stack options: licenses, transports, plaintext recipients, privacy choices and pending live evidence |
| [Codemode and Plugins](plugins.md) | Scripts that call plugin tools, MCP plugins and their installs, sign-in, plugin calls at turn time |
| [Skills](skills.md) | Bounded metadata discovery, Runner and desktop discovery contract, managed copies, provenance and uninstall |
| [Marketplace](marketplace.md) | The relay-backed index of plugins and bot templates, offline fallback and updates, bots added from a template, the marketplace sheet |
| [MCP servers](mcp-servers.md) | The user's own MCP servers in a Runner's `mcp.json`: the file and other apps' spellings, sign-in, the `mcp.*` methods and `beans mcp`, the apps' MCP Servers section and server sheet |
| [Bots, Routines, and Memory](bots.md) | The lead bot, DMs and groups, group descriptions and ownership, who answers, handoffs between bots, routines and their checks, a bot's memory |
| [Memory service UI](memory-ui.md) | Client drafts, masked replies, secret/options patches, consent and capability gates |
| [Memory services](memory-services.md) | Encrypted config, dispatch, queues, deletion and embeddings |
| [Bot avatars](avatars.md) | Generated appearance contract, independent photos, validated API edits, encrypted persistence and fresh-format protocol compatibility |
| [Providers](providers.md) | Each model provider and its sign-in, custom providers, thinking levels, compaction, retries |
| [Zhipu GLM compatibility](glm-compatibility.md) | Vendor documentation, existing custom-wire evaluation, unsupported reasoning-tool continuation and local transport evidence |
| [Model catalog and cost](model-catalog.md) | Shared model metadata, pricing completeness, usage, and public catalog refresh |
| [macOS app](macos-app.md) | AppKit launch, windows, onboarding, settings, updates, inspector, Blobatar |
| [macOS sidebar](macos-sidebar.md) | Sidebars, search, native chrome, toolbar navigation |
| [macOS chat](macos-chat.md) | AppKit transcript, composer, attachments, dictation, and working state |
| [Composer capabilities](composer-capabilities.md) | Proposed context/action consumers and their implementation boundaries |
| [Composer](composer.md) | Current client/catalog inventory and selected discovery-only action/reference syntax, authority, compatibility, preservation and capability dispositions |
| [Windows and Linux app](desktop-app.md) | The MyGo app: its Go side and Solid page, title bar, commands, updates, development and builds |
| [Phone app](phone-app.md) | The Expo app over the Rust core: the native module, pairing, relay status, attachments, dictation, notifications, turns |
| [Phone releases and updates](phone-updates.md) | TestFlight builds, Android APK releases, and the Android app's self-update: the native gate, daily check and offer, the progress dialog, the package installer |
| [Phone interface](phone-interface.md) | Native screens, transcript presentation and adaptive navigation |
| [Releases](releases.md) | Signed readiness, desktop/EAS builds, testing submissions, accounts and credentials |
| [Website](website.md) | `web/`: the site, its docs, and the install scripts it serves |
| [Languages](languages.md) | English and Simplified Chinese in each app, and what the CLI words |
| [Portable setup](portable-setup.md) | Secret-free configuration recipe boundaries, versioning, ownership, validation and import transaction contract |

| [Relay trust](relay-trust.md) | What a Device takes from the relay and how it checks it: Device records and unpaired keys, signed envelopes, the challenge nonce, the pairing reply key, the account key's generations and their rotation on unpair |
| [Requests between Devices](requests.md) | What one Device asks a Runner through the relay, sealed: timeouts and each verb |
| [Notifications](notifications.md) | Phone pushes for replies, failures, and questions, the desktop app's own notifications, reads on another Device, the chat on screen |
| [Bot permissions](bot-permissions.md) | A bot's Access to plugins, tools, files, and shell; where the CLI checks it; access requests |
| [Auto-review](auto-review.md) | Fast paths, what the review reads, its rules and two passes, holds back to the bot, asks |
| [Handoffs](handoffs.md) | Durable delegated contracts, expected outputs, result evidence and return routing, offline delivery and restart recovery |
| [Review queue](review-queue.md) | Encrypted editable proposals, version-bound approval, Runner execution and outcomes |
| [Message drafts](drafts.md) | Emails and Slack messages a bot writes in a chat as drafts the user edits, sends, or discards; sending directly; edits as feedback |
| [Outputs and evidence](outputs.md) | Bot-generated files and document links, immutable versions, task evidence references, encrypted transport and native previews |
| [Stop](stopping.md) | What Stop ends: the turn's model call and tool calls, a call that never returns, commands and browser calls, the work it handed off on any Runner; what keeps running |
| [Coding agents](coding-agents.md) | Claude Code, Codex, OpenCode, pi, or fx a bot runs on its Runner: which one, its folder, protocols, secrets, review, transcript steps, pull request state, its message and the bot's wake-up, the apps |
| [Coding agent roster](agent-roster.md) | Every coding agent on the account, listed on each Device from its own store: for the bot, a start beside another agent, and the apps |
| [Secret requests](secrets.md) | A password, key, or code a bot asks for on a card: sealed to its Runner, kept there, filled into Browser on its site or a command's environment by name, kept out of results and chats; managing a Runner's secrets |
| [Browser sessions](browser-sessions.md) | A bot's browser profiles on its Runner: their sign-ins, opening one in a window, taking the browser over and handing it back, recording a workflow and running it as a skill's steps, screenshots in the chat, what works from another Device, the Profiles section |
| [Integrations and named accounts](integrations.md) | Slack, Gmail, Calendar, and Drive; stable account instances, OAuth on the Runner, account selection, access recovery, and source links |
| [Workflow onboarding](workflows.md) | Workflows from the marketplace: packs, a setup on a Runner and its accounts, the sample before the schedule, how setups sync, the workflow page in each app |
| [Bot templates](templates.md) | A bot shared as a link or a file and a new bot made from one: what a template holds, redaction and flags, encrypted links and their keys, the Runner's own connections, the apps' sheets |
| [Routines](routines.md) | A bot's scheduled tasks: schedules and their timezones, runs and read-only checks, missed runs, health and recovery, and the apps' routine sheet and service row |
| [One-time routines, routines on events, and calendar events](routine-triggers.md) | Routines that run once at a date and time, run for each event about one subject (a pull request until it merges or closes, a webhook's requests), or run around a Calendar account's events |
| [Event triggers](event-triggers.md) | Runner event subscriptions, the gateways users run to sign and seal a service's events, the encrypted inbox, ordering and recovery |
| [Hosted receivers](receivers.md) | The relay's receivers for routines on events: the Beans GitHub App and its install and bind, routine webhooks with their URL and key, what the relay sees, setup |
| [Channels](channels.md) | A bot listening on Telegram and Slack: the accounts and their builtin servers, filters, the chats the user allows and the ones asking, conversations and what people write there, replies, the feedback collector |
| [Channel readers](channel-readers.md) | How the Runner reads Telegram and Slack, what it takes, Telegram's photos and albums |
| [Coordinator attention](attention.md) | Consolidated reviews, blockers, commitments and changes, coordinator briefs, deduplication, encrypted records and notification preferences |
| [Shared project context](project-context.md) | Group briefs, goals, constraints, decisions, source freshness and corrections, encrypted reference assets, bounded bot discovery |
| [Playbooks](playbooks.md) | Skills users write for a bot or group, saving one from a chat, revisions, discovery, export |
| [Workflow feedback](feedback.md) | Feedback on bots' work, suggested changes to routines and skills, versions and undo, exclusions |
| [Compaction and usage](compaction.md) | Summarizing a long chat, Compact, chat usage on every Device |
| [Budgets and connector limits](budgets.md) | Limits on turns, tasks, and routines, stopping and resuming, price labels, shared plugin call limits |
| [Sidebar sections and alerts](sidebar.md) | Sections, hidden and muted chats in every app, and the chat a computer shows in front, which no phone is alerted about |
| [Desktop app updates](desktop-updates.md) | The desktop app's updater and releases, and an update another Device starts |
| [Phone layout](phone-layout.md) | The phone's home tabs (Bots, Devices, Search) and a tablet's sidebar and pane |
| [Repo layout and development](development.md) | Where each crate and app lives, the desktop, phone, and Android development loops, a local relay |
| [Status](status.md) | What is done, what is next, and the open points |

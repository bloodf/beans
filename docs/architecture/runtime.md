# CLI (runtime)

The local websocket retains Host validation and adds service-token authentication at upgrade. Browser Origin rejection is an additional boundary. A browser development allowlist alone does not authorize a websocket connection.

Local storage, account format admission, staged scratch preload and mutable startup persistence are described in [Local storage and startup admission](storage-admission.md).


Commands:

- `beans tasks list` / `get <id>` / `create '<JSON>'` / `update '<JSON>'` / `run '<JSON>'` — [durable work](durable-tasks.md), through the local service for mutations; `-` reads JSON from stdin. Encrypted records, receipts, and run journals share `beans.sqlite3`, and interrupted claims require explicit recovery.

- `beans serve` — the app connects here; `beans` alone lists the commands and starts nothing, so a bot's `beans` never starts a second service
- `beans identity new` / `identity restore <phrase>` / `identity show` / `identity phrase` (the backup phrase again, on the identity device)
- `beans pair` — show a pairing string and wait; `beans pair <string>` joins
- `beans provider set <kind> [api-key] [--base-url <url>]` — connect DeepSeek, Anthropic, OpenCode Zen (`opencode`), or OpenCode Go (`opencode-go`) with an API key, or open the browser sign-in for `chatgpt` (`grok` production OAuth rejects before callback/network pending a verified Beans contract). The key is checked against the API first and read from stdin when omitted. `provider add <name> <base-url> [--api chat-completions|responses|messages] [--model <id>]… [--api-key-stdin]` adds a custom provider, its server checked first; `provider remove <kind>` (a custom provider's kind deletes it) / `provider list`. A running `beans serve` carries out the change over the local websocket; with none, the command changes `credentials.json` itself and runs one sync pass
- `beans mcp list|get|add|add-json|remove|enable|disable|hide|show|sign-in|sign-out|reload|import` — manage [custom MCP servers](mcp-servers.md) in this Runner's private `mcp.json`, through a running local service when available. `mcp add` accepts a command and arguments or a URL, `--transport stdio|http`, repeated `-e`/`--env NAME=VALUE`, repeated `-H`/`--header "Name: value"` for HTTP, `--description` and `--timeout` in seconds. Import skips existing names; explicitly named invalid files or entries fail. List and reload connect enabled servers before reporting health and exit nonzero for invalid or unready servers; disabled servers do not fail health checks.
- `beans marketplace reload` / `beans models reload` — force a check of the configured public feed through the running service, or update its source-bound cache for the next startup. Bundled catalogs remain available offline.
- `beans chats list` / `beans chats set-owner <group> <bot>` — list chats and group owners, or choose a member as owner by case-insensitive title/name or exact id. Ambiguous names require an id. Changes reach the running service over its websocket, or update the local store and run one sync pass.
- `beans status` — account snapshot for local inspection, not a support export.
- `beans doctor` / `beans doctor --json` — the same privacy-safe setup report, rendered as text or JSON. The existing invocation and home/port defaults remain unchanged. Both outputs deliberately disclose the selected relay host and explicit port; userinfo, URL paths, query strings, fragments, account names, keys, local paths, message content and raw errors are omitted. Review before sharing; nothing is uploaded. Only the relay's public health endpoint is probed. Provider checks mean configured credentials, not verified authentication or reachability. Plugin slots are anonymous, report-local ordinals with cached setup states, not live connection checks. Runner rows include only desktop Devices; presence is not admission readiness. Paired versions appear only when they are numeric three-part releases.
- `beans service install` / `uninstall` / `status` — keep `beans serve` running from login on ([Service management](service-updates.md#service-management)).
- `beans self-update` — reports that CLI self-update is unavailable in Beans, before opening account data or making network requests.

The hidden global `--message-format <value>` option is accepted and ignored for Cargo-wrapper compatibility, including when appended after `serve` arguments. It does not change output formatting; unrelated unknown flags remain errors.

Bind: `127.0.0.1:4874` (Beans) or `127.0.0.1:4875` (Beans Dev), selected by `--port` / `BEANS_PORT`; native builds fix their matching isolated port. Relay: `BEANS_RELAY_URL`. A busy port fails loudly.

The `/ws` upgrade checks `Host` before opening the socket: only `localhost:<serve port>` or `127.0.0.1:<serve port>` is accepted, even though the listener binds to loopback. Native clients (the AppKit app and the Go desktop client) send no `Origin` and connect with either host. Browser requests with an Origin are rejected by the service middleware. The websocket also requires a valid local service Bearer token, compared in constant time. Host validation remains required before upgrade.
Outgoing requests (providers and their sign-ins, the relay, MCP servers over HTTP) use the proxy in `HTTPS_PROXY`, `HTTP_PROXY`, or `ALL_PROXY`, less the hosts in `NO_PROXY`, else the system's: the HTTP and HTTPS proxies in macOS's network settings, or Windows' Internet Options with their bypass list (reqwest's `system-proxy`). So the CLI reaches a provider the way the browser does behind a proxy app set as the system proxy, and a ChatGPT sign-in whose browser half went through the proxy swaps its code through it too; OpenAI refuses the swap from a country it does not serve. Neither a PAC file nor macOS's bypass list is read, so on a Mac with a system proxy a model or MCP server on the computer itself is reached through the proxy as well.

Over HTTPS, those requests, public catalog requests, and the relay's sync socket check certificates with one rustls config per process, from `crates/tls` (`beans-tls`). On macOS and Windows the system decides, through rustls-platform-verifier: the Keychain or the Windows certificate store, as in the browser, so a root an administrator installed for a TLS-inspecting proxy such as Zscaler is trusted. `SSL_CERT_FILE` there names a PEM file of more roots to trust beside the system's; a file that does not read is logged and skipped. On Linux the CLI trusts the system's CA store, or what `SSL_CERT_FILE` and `SSL_CERT_DIR` name in its place (rustls-native-certs), together with the Mozilla roots it carries (webpki-roots), so a container without a CA store still connects. The phone core trusts the carried roots alone. `reqwest::Client::new()` would trust only reqwest's bundled roots, so Device code builds its clients with `beans_tls::client()` or `client_builder()` (HTTP/1.1); the MCP client, on reqwest 0.13, takes `client_config` and offers h2. A provider check or turn whose request fails reads `beans_tls::describe`: reqwest's words, then the last cause in the chain, so a certificate the system does not trust says `invalid peer certificate` where a refused connection says so.

Installation, standalone services and Runner admission drain are described in [CLI service and updates](service-updates.md).

`beans serve` loads an execution owner with an exclusive `execution.lock` held for the App lifetime before opening SQLite or recovering work. Other `App::load` callers do not recover submitted memory deliveries. Task admission and interrupted-work fences are described in [Local task authority](tasks.md). Supported populated legacy stores remain available for serving, reading and sync. Explicit local user submissions mint individually fenced new task identities; ambiguous sealed or automatic legacy work stays closed. Sender timestamps never grant admission. Opening a Device view does not grant execution authority.

Local user submissions commit transcript, encrypted outbox and exact first-turn task admission together before publishing or steering. Attachments use private staging and rollback backups; filesystem finalization and SQLite commit are separate failure boundaries, described in [Local task authority](tasks.md#admission-and-recovery).

Future routine ticks require explicit local `routines.reauthorize` authority under the current execution owner. Creation, editing and resume do not authorize automatic execution. Exact revision and unique occurrence admission share the durable staged check/task transaction; missed occurrences are skipped, not caught up. Pause, Runner, membership or definition changes revoke schedule admission. Pending or unknown matching intent blocks reauthorization. See [Local task authority](tasks.md#routine-checks-and-retention).

The staged `history164_types` module provides closed history event and producer validation, bounded list/export response codecs, and canonical authenticated cursors. Its HMAC helpers borrow an existing purpose key and host-supplied account namespace, distinguishing `LocalTaskEpoch` from `ImportedAccount` without creating task authority. Response validation binds the cursor to the filter, watermark, last-scanned position and captured host context. This pure module is not connected to live history handlers or SQL storage and performs no filesystem export.

Linux CI compiles `beans` library tests with `--no-default-features --lib --no-run`; relay ownership fixtures run with `server` enabled.

The Windows Rust verification job builds the real relay with `cargo build --locked -p beans-relay --bin beans-relay` before its selected-package tests. It resolves `debug/beans-relay.exe` under Cargo metadata's `target_directory`, checks that the executable exists, and exports its absolute path as `BEANS_REVOCATION_RELAY_BIN` through `GITHUB_ENV`. This respects `CARGO_TARGET_DIR` and supplies the revocation fixture's scratch relay without building inside the fixture or excluding Windows coverage.

## Serve shutdown

`beans serve` starts work it does not own: sync, routines, the plugin and MCP starts, the marketplace check, the login-shell environment, the model refresh and the local websocket, plus per-request handlers inside it. Before any of them start, `serve::run` records each of those boundaries as work with no known end, so the process never claims it knows when their writing stops. Only the startup model-catalog check is owned: `catalog::check_in_background_owned` holds a permit for it, and the drain joins that permit.

A shutdown — SIGTERM, SIGINT, SIGHUP, or the app that started the CLI going away — stops the commands bots left running in their terminals, cancels the turns in flight, and joins owned work, all under one deadline (10 s). Then the process ends exactly as it did before: a signal still ends it by that signal, and a parent that went away still exits 0. Refusing a final flush changes nothing about how the process ends, only that nothing is written at the end.

This is a bounded drain, not quiescence. Work with no known end is still running when the process ends, untracked interiors (a plugin's MCP server, a plugin's sign-in, the login-shell cache) are untouched, and a turn stopped mid-flight stays as it was recorded: a job sent to another Runner whose wait is cancelled loses its row and is not replayed. Tracking those boundaries is later work, and until it lands no shutdown writes a final state.

## Diagnostics report

`diagnostics.report` and `beans doctor --json` use the same builder. The report contains `schema_version: 1`, `versions {core, relay_protocol}`, `this_device {os, is_runner, has_identity}`, `home {exists}`, `port {free}`, `relay`, `providers`, `plugins`, `mcp_json` and `runners`. It uploads nothing; consumers must let the user review before sharing.

If local account loading fails, either doctor form exits nonzero, emits no report on stdout, and returns a fixed privacy-safe error on stderr. Doctor omits startup logs and the underlying error chain, including private paths and record values. Storage access and account format must be checked locally; account admission still rejects unreadable, malformed and unmarked populated homes without resetting them. Other commands retain their existing startup error handling.

- `relay {configured, host, reachable, reason, http_status, protocol}` deliberately discloses only the selected host and explicit port. `reason` is `none`, `not_configured`, `invalid_url`, `unreachable`, `update_required` or `http_error`. No raw response/error text enters the report. `reachable` is null when no probe runs, false for transport failure, true when health supplies HTTP evidence. The existing health interface represents malformed/wrong-service/incompatible successful responses as 426; `update_required` does not distinguish those cases from HTTP 426. Health 404 is `http_error`, never an unknown Device. `protocol` contains validated advertised protocol only on success; the interface supplies no relay version.
- `providers {built_in: [{kind, configured}], custom_configured, health: "not_checked"}` contains only fixed built-in kinds and a custom connection count. Custom names/kinds/endpoints and credentials are omitted. Configured does not mean working authentication or reachability. No credential-bearing provider calls run.
- `plugins {basis: "cached_setup_state", entries: [{slot, state}]}` uses report-local one-based slots, not stable identities. State is `ready`, `needs_setup`, `needs_auth`, `connecting`, `error` or `unknown`; arbitrary states/details/names are omitted. Anonymous slots cannot identify a private plugin across reports. No plugin connection is started.
- `mcp_json {servers, problems, file_error}` counts configuration problems, including invalid entries, without echoing validation text or file paths.
- `runners [{is_this_device, os, presence, version}]` excludes phones/tablets. Presence is `online` or `offline`, not work admission/readiness. Untrusted paired version text is null unless it is a numeric three-part release. OS values are allowlisted, with `unknown` for unrecognized local values. A busy local port is a note, not proof of a failed service.

`beans status` and snapshots contain private account data and are not substitutes for this report. The report describes configured providers, cached plugin setup and Runner presence, not verified provider health or work readiness.

Each app's Devices pane shows whether the service keeps a standalone Runner's CLI running: `device.service_status { id? }` answers `{ installed, running }` for this CLI, or asks another Runner with the sealed `service.status` request. The apps identify released standalone CLIs by the `update` they advertise in their `machine` blob, even with automatic updates off. Only these Runners are asked for service status and show the Background service row; while it is not installed, the pane says to run `beans service install` in Terminal on that computer. The desktop app bundles and launches its own CLI without installing a `beans` command on PATH, so its Runners show neither the service row nor its Terminal instructions.

## Agent loop

See [Agent runtime](agent-runtime.md#agent-loop).


## Updates and the service

See [CLI service and updates](service-updates.md). Standalone CLI self-update is unavailable. Remote app updates require the client readiness and renewable drain admission described there.

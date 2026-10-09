# Local API

The app speaks to `beans serve` over JSON on `ws://127.0.0.1:<port>/ws`; the CLI subcommands and the phone core call the same `api::dispatch` (`crates/cli/src/api.rs`) directly. [Protocols](protocols.md) covers the relay side; this doc is the method, event, field and trust reference for the local side.

## Connection and envelope

- Request `{ "id": <any JSON>, "method": "<name>", "params": {…} }` (`params` defaults to `null`). Reply `{ "id", "result" }` or `{ "id", "error": { "message": "<text>" } }`. Text that is not a request object answers `{ "id": null, "error": { "message": "bad request: …" } }`. An unknown method answers `unknown method <name>`.
- Events are `{ "event": "<name>", "data": {…} }`, sent to every connected socket, never filtered. A client that falls behind receives a fresh `snapshot`.
- Replies are unordered: each request runs in its own task, so match them by `id`.
- Versions: `hello` answers the CLI `version`. The socket has no version negotiation; a client detects a missing method by `unknown method`. Relay protocol 5 and format `beans-v2` are relay evidence, shown by `relay_update_required`, not a socket version.
- Example frames, with ids chosen by the caller:

```json
{"id":1,"method":"hello","params":{}}
{"id":2,"method":"chats.send","params":{"chat_id":"<chat id>","text":"Hello"}}
```

`beans status` prints the same masked snapshot `bootstrap` answers, with no socket needed.

## Trust boundary

`check_upgrade_headers` (`ws.rs`) runs before the socket opens and answers `403` on failure:

- `Host` is exactly one header, exactly `localhost:<port>` or `127.0.0.1:<port>`.
- `Origin` absent: allowed (AppKit, the Go desktop client, the phone core, the CLI).
- `Origin` present: exactly one header and an exact origin, with no path, wildcard, userinfo or list. It must equal `http://<Host>` or appear in `BEANS_ALLOWED_ORIGINS`, a comma-separated list of exact origins read on each upgrade; one malformed entry rejects every cross-origin upgrade.

There is no per-connection credential, no per-method authorization and no sandbox. The boundary is the local user account: any local process that connects without an `Origin` can call every method below, including the secret-bearing and destructive ones, and a page served from the CLI's own `http://localhost:<port>` or `http://127.0.0.1:<port>` origin passes the `Origin` check. Only `update.prepare` and `update.cancel` check a token: the contents of the file `BEANS_UPDATE_TOKEN_FILE` names, at least 32 characters, in a regular file of at most 4096 bytes that is not a symlink, is owned by root or the running user, and is not group-writable or readable by others (on Windows, a private owner and DACL); it is compared by digest. `update.status` needs none. See [Runtime](runtime.md) for the listener and proxy behavior.

## Field errors

The required string parameters that `api.rs` reads through its `string` helper (`chat_id`, `bot_id`, `id`, `name`, `runner_id`, `plugin_id`, `kind`, `schedule`, `query`, `phrase`, `nonce`, `pairing_string`, `platform`, `token`, `message_id`, `decision`, `api_key` and `text` of `mcp.parse`) answer the same two errors. `chats.send` validates fields and admission before storing attachments. `bots.update` checks bot existence before avatar storage. Other methods check fields in their own order. The `mcp.*` runner verbs read `name`, `enabled`, `tool` and `hidden` themselves and answer `missing <field>` for a wrong type; `memory.*` bodies answer the codes listed under Memory methods; `update.*` answers its own sentences.

| Case | Error |
| --- | --- |
| Required string absent, `null` or `""` | `missing <field>` |
| Required string of another JSON type | `<field> must be a string` |
| Optional string read as `opt_string` (most optional fields) of another type or `""` | read as absent |
| `chats.send`, `text`, `message_id` or `reply_to` present and not a string or `null` | `<field> must be a string` |
| `chats.send`, no `text` (absent or `null`) and no `attachments` | `missing text` |
| `chats.send`, `attachments` present and not an array of `{ path, id?, name?, mime?, width?, height? }` | `attachments must be an array of files` |
| `chats.send`, `mentions` present and not an array of strings | `mentions must be an array of strings` |

`chats.send` checks `chat_id`, chat existence (`Unknown chat`), optional string types, attachment and mention shapes, and missing text before runtime admission. Runtime checks blank text (`Empty message` without attachments), chat existence, quotable `reply_to` and update drain before file preparation. Attachment-only sends remain valid. Absent and `null` mean "not given" except for `chat_id`. Every source is read into private staging before any attachment id is replaced; a rejected source leaves existing bytes, messages, queued blobs and jobs unchanged. Successful preparation moves staged files into `files/`, queues their blobs, then stores the message. This is not a filesystem transaction: a destination I/O failure during commit can leave earlier files committed; concurrent file or chat changes are not guarded.

## Methods

`[R]` needs the `runner` feature, `[PA]` the `provider-auth` feature. "Secret" marks a method that takes or returns a credential; see [Secret boundaries](#secret-boundaries). `?` marks an optional field.

### Identity, Devices, sync

| Method | Params | Result |
| --- | --- | --- |
| `hello` | none | `version`, `has_identity`, `is_identity_device`, `device_id`, `relay_url`, `relay_connected`, `relay_update_required`, `relay_error` |
| `diagnostics.report` | none | schema-1 privacy-safe setup report (see [Diagnostics report](runtime.md#diagnostics-report)); probes public relay health only |
| `bootstrap` | none | snapshot (see `snapshot`); starts the hourly catalog and marketplace checks |
| `identity.create` | `device_name?` | `phrase` (secret), `device_id` |
| `identity.restore` | `phrase` (secret), `device_name?` | `device_id` |
| `identity.forget`, `identity.delete` | none | `null`; `delete` removes the account at the relay first and fails if it cannot |
| `pair.start` | none | `nonce`, `pairing_string` |
| `pair.status` | `nonce` | `{state:"waiting"}`, `{state:"completed",device}` or `{state:"failed",error}` |
| `pair.cancel`, `pair.abort` | `nonce` (cancel only) | `null` |
| `pair.accept` | `pairing_string`, `device_name?` | `id`, `name`, `os`, `identity_id` |
| `device.rename` | `name` | `name` |
| `device.unpair` | `id` (this Device's id forgets the identity) | `null` |
| `device.update`, `device.auto_update` | any | always the error `SELF_UPDATE_UNAVAILABLE` |
| `sync.account` | none | `providers` (masked); waits up to 30 s for the pull |
| `sync.wake`, `ui.watching` | `chat_id?` (watching) | `null` |
| `push.register` | `platform`, `token`, `environment?` | `null` |
| `push.unregister` | none | `null` |
| `config.set` | `relay_url?` | `relay_url` |
| `account.pause` | `paused` (bool; another type answers `paused must be a boolean`) | `paused` |
| `update.status` | none | `version`, `pid`, `control`, `prepared`, `expires_in`, `ready`, `idle`, `active`, `jobs` |
| `update.prepare` | `token`, `ttl?` (whole seconds, clamped 30–3600, default 600) | `update.status` fields plus `lease_id` |
| `update.cancel` | `token` | `released` |
| `models.reload`, `marketplace.reload` | none | `updated`, `changed` |

### Bots and chats

| Method | Params | Result |
| --- | --- | --- |
| `bots.create` | `name` or `template_id`, `runner_id`, `id?`, `chat_id?`, `description?`, `symbol_name?`, `accent?`, `provider?`, `model?`, `thinking?`, `workdir?`, `instructions?` (legacy), `capabilities?` `{shell?,write?,plugins?}`, `look?`, `avatar?`, `greeting?` | `bot`, `chat_id` |
| `bots.update` | `id`, any create field except `template_id` | `bot` |
| `bots.delete` | `id` | `null` |
| `bots.memory` | `bot_id` | the bot's `MEMORY.md` overview, `bot_id`, `here`, `runner` (plaintext memory) |
| `bots.memory.write` | `bot_id`, `text`, `expected_hash?` | `hash` |
| `chats.create` | `bot_ids`, `kind?` (`group` default, or `dm`), `id?`, `title?`, `owner_bot_id?`, `description?` | `chat` |
| `chats.dm` | `bot_id`, `id?` | `chat` |
| `chats.send` | `chat_id`, `text?` (required unless `attachments` is not empty), `message_id?`, `attachments?`, `mentions?`, `reply_to?` | `message` |
| `chats.send_now` | `chat_id`, `message_id` | `sent` |
| `chats.stop`, `chats.delete`, `chats.mark_read` | `chat_id` | `null` |
| `chats.compact` [R] | `chat_id`, `bot_id?` | `tokens_before` |
| `chats.rename` | `chat_id`, `title?` | `null` |
| `chats.set_description` | `chat_id`, `description?` | `null` |
| `chats.pin` | `chat_id`, `pinned?` (toggles when absent) | `null` |
| `chats.add_bot`, `chats.remove_bot`, `chats.set_owner` | `chat_id`, `bot_id` | `null` |
| `chats.search` | `query`, `limit?` (1–50, default 20) | `chats`, `messages` |
| `chats.messages` | `chat_id`, `before?`, `limit?` (1–200) | `messages`, `has_more` |
| `chats.permission` | `chat_id`, `message_id`, `decision` (`allow`, `always`, `deny`) | `answered` or the plugin sign-in reply |
| `bash.stdin`, `bash.stop`, `bash.background` | `chat_id`, `message_id`; `bash.stdin` also `text?`, `enter?` | the Runner's reply |
| `files.path` | `attachment` | `path` (the decrypted file on this disk) |
| `routines.create` | `bot_id`, `name`, `schedule`, `prompt?`, `enabled?` | `routine` |
| `routines.update` | `id`, `name?`, `schedule?`, `prompt?`, `enabled?` | `routine` |
| `routines.delete`, `routines.run` | `id` | `null` |
| `routines.reauthorize` | `id` | `routine` |
| `routines.describe` | `schedule` | `schedule`, `text`, `next_run_at` |
| `auto_review.set` | `is_enabled?`, `rules?` | `auto_review` |

Local-only [schedule authority](tasks.md#routine-checks-and-retention) needs explicit reauthorization.

### Local task history

`tasks.list` accepts null or an object with only `limit?` and `cursor?`. Omitted or null limit defaults to 50; a supplied integer must be 1–100. Cursor is null or a nonempty opaque string of at most 2048 bytes. The CLI uses canonical URL-safe unpadded base64 over its private cursor representation and validates the decoded account epoch and task-id keyset before querying tasks. Decoded ids and returned structural ids are nonempty, at most 256 UTF-8 bytes and contain no control characters. Invalid fields, bounds or cursors reject with fixed text, without echoing input or storage errors. Clients pass cursors back unchanged; encoding provides neither secrecy nor authorization.

Result is `{tasks:[{task_id,chat_id,bot_id,routine_id,state}],next_cursor}`; `routine_id` and `next_cursor` may be null. State is `queued`, `running`, `finished`, `interrupted` or `needs_review`. Pages use ascending task-id keysets within the current nonclosed account epoch. A different or missing epoch rejects a supplied cursor; no epoch with no cursor returns an empty page without creating authority. Cursor is a position, not permission. This method uses the existing local-user trust boundary, reads only this home's history, and has no sealed remote alias, Runner routing or sync projection. See [Local history reads](tasks.md#local-history-reads).

### Plugins, MCP, providers

| Method | Params | Result |
| --- | --- | --- |
| `marketplace` | `query?` | `plugins`, `bots` |
| `plugins.install` | `runner_id`, `plugin_id` or `manifest` | `status` |
| `plugins.uninstall`, `plugins.detail` | `runner_id`, `plugin_id` | `null` for uninstall; for detail, the plugin, which variables are set (never their values) and each server's state, including a sign-in `code` and `link` while one waits |
| `plugins.set_variables` | `runner_id`, `plugin_id`, `variables` (secret values) | `status` |
| `plugins.connect` | `runner_id`, `plugin_id`, `server?` (the Runner owns its loopback callback) | `message`, `url`, `sign_in` |
| `plugins.sign_out` | `runner_id`, `plugin_id`, `server?` | plugin detail |
| `plugins.auth.cancel` | `sign_in?` | `null` |
| `mcp.parse` | `text` | parsed servers |
| `mcp.list`, `mcp.reload` | `runner_id?` | `path`, `error`, `servers` |
| `mcp.get`, `mcp.remove`, `mcp.sign_out`, `mcp.reconnect` (`fresh?`, default true), `mcp.sign_in` (`wait?`) | `runner_id?`, `name` | `get`, `reconnect`, `sign_out` and a waited `sign_in`: `path`, `server`; `remove`: `null`; `sign_in` otherwise: `message` |
| `mcp.save` | `runner_id?`, `name`, `config`, `previous_name?` | `path`, `server` |
| `mcp.set_enabled` | `runner_id?`, `name`, `enabled` | `path`, `server` |
| `mcp.hide_tool` | `runner_id?`, `name`, `tool`, `hidden` | `path`, `server` |
| `providers.api_key` | `kind` | `api_key` (secret), `base_url` |
| `providers.connect_deepseek`, `_anthropic`, `_opencode`, `_opencode_go` [PA] | `api_key` (secret), `base_url?` | `providers` (masked) |
| `providers.connect_custom` [PA] | `name`, `base_url`, `api_key`, `models`, `api?`, `kind?`, `integration?` | `kind`, `providers` |
| `providers.connect_chatgpt`, `providers.connect_grok` [PA] | none | `email`, `providers`; `connect_grok` is disabled in production builds |
| `providers.list_models`, `providers.refresh`, `providers.auth.cancel`, `providers.disconnect` [PA] | `name`, `api`, `base_url`, `api_key`, `integration?` (list); `kind` (disconnect) | `listed`, `models`; `updated`; `null`; `providers` |

Every `mcp.*` result that holds a server carries its full `config`, which is secret-bearing; see [MCP servers](mcp-servers.md#managing-it).

`memory.read` and `memory.write` have no local arm and answer `unknown method`; they are sealed verbs only (`bots.memory` and `bots.memory.write` are the local calls).

## Memory methods

The methods match [Memory services](memory-services.md) and [Memory UI](memory-ui.md). Except `memory.connections.list`, which ignores its params, every body is a strict object: an unknown key, a wrong type or a missing field answers `invalid_memory_request`. A service or operation call reads `bot_id` first, so a missing or non-string `bot_id` there answers `missing_bot_id`. Failures are codes, not sentences. An `id` is 1–1024 bytes with no control character. A Revision is `{ counter: u64, device_id }`.

### Configuration (this Device)

| Method | Params | Result |
| --- | --- | --- |
| `memory.connections.list` | none | `schema_version`; `connections` (`id`, `revision`, `backend`, `name`, `has_secret`, `embedding_profile`, `availability` `supported` or `blocked`, `reason`); `embeddings` (`id`, `revision`, `model`, `model_revision`, `dimensions`, `has_secret`); `bots` (`bot_id`, `preferences`) |
| `memory.connections.set` | `id`, `backend` (`hindsight`, `open_viking`, `pgvector`, `lance_db`), `name` (1–256), `secret`, `endpoint?`, `embedding_profile?`, `allow_insecure_http?`, `options?` | `saved`, `setup_required` |
| `memory.connections.disconnect` | `id` | `saved` |
| `memory.embeddings.set` | `id`, `profile`, `secret` | `saved` |
| `memory.embeddings.remove` | `id` | `saved` |
| `memory.preferences.get` | `bot_id` | preferences |
| `memory.preferences.set` | `bot_id`, `auto_recall`, `capture_conversation`, `capture_group_text`, `connection_id?`, `unattended_capture?`, `max_capture_deliveries_per_turn?`, `recall_budget?` | preferences |

Shared shapes:

- `secret` is `{action:"keep"}`, `{action:"clear"}` or `{action:"replace",value}` with `value` 1–8192 bytes. Reads return only `has_secret`.
- `endpoint` and `embedding_profile` omitted keep the saved value; `null` clears it. The same holds for `options` (omitted keeps). An endpoint has no userinfo, query or fragment. Hindsight and OpenViking need `https`, or `http` with `allow_insecure_http`; pgvector needs `postgres` or `postgresql`; LanceDB needs `db`, and a Cloud (`endpoint`) connection answers `lancedb_cloud_transport_unavailable` when selected or initialized.
- `options` is tagged by `backend`, which must equal the connection's: `{backend:"hindsight"}`; `{backend:"pgvector",schema,role?}` (`schema` 1–63 of `[A-Za-z0-9_]`, `role` 1–63); `{backend:"lance_db",region}` (`region` null or 1–128); `{backend:"open_viking",bindings,replace_all?,confirm_replace_all?}` where `bindings` maps a bot id to `null` (remove) or `{mode:"user_key"|"trusted_gateway",account_id,user_id,secret}` (ids `[A-Za-z0-9_-]`). `replace_all` without `confirm_replace_all` answers `confirmation_required`.
- `profile` is `{model,revision,dimensions,normalization,distance,document_prefix,query_prefix,endpoint?,mode?,local?}` with `mode` `api` (default) or `local_cpu`. It has no `secret` key: an inline one answers `secret_patch_required`. At use, `model` and `revision` are 1–1024 bytes, `dimensions` 1–65536, prefixes at most 4096 bytes, `normalization` `none` or `l2`, `distance` `cosine`, `dot` or `euclidean`. `api` needs an `endpoint` ending in `/embeddings` and no `local`; `local_cpu` needs `local` `{model_sha256,tokenizer_sha256,max_tokens (1–8192),pooling (mean|cls|pooled),tensors{input_ids,attention_mask,token_type_ids?,output},add_special_tokens,pad_id,pad_type_id,pad_token (≤256)}`.
- Preferences are `connection_id`, `auto_recall`, `capture_conversation`, `capture_group_text`, `unattended_capture`, `max_capture_deliveries_per_turn`, `consent_revision`, `deletion_epoch` and `recall_budget`. `connection_id` omitted keeps, `null` clears. `max_capture_deliveries_per_turn` is 0–4. `recall_budget` is `{timeout_ms 1–5000, max_bytes 1–32768, max_results 1–20, max_context_chars 100–8000}`. `unattended_capture: true` answers `enforced_spending_policy_required`.
- Other codes: `bot_deleted`, `connection_disconnected`, `invalid_connection`, `invalid_budget`, `identity_required`, `unknown_memory_method`.

### Service and operations (the bot's Runner)

Each carries `bot_id`; another Device receives it as a sealed request to the bot's Runner, which waits 20 s for the answer. A call that times out may still run there: read `memory.operations.status` before a retry. An unknown bot answers `bot_deleted`, a missing or non-string `bot_id` `missing_bot_id`. Admission rechecks account Pause, assignment, connection revision, consent and deletion epoch (`runner_draining`, `account_paused`, `runner_changed`, `authority_changed`).

| Method | Params (beside `bot_id`) | Result |
| --- | --- | --- |
| `memory.service.health` | none | `status` (`ready`, `degraded`, `setup_required`), `capabilities`, `deletion_pending` |
| `memory.service.recall`, `.reflect` | `query` (1–4096 characters) | `evidence` [`id`,`text`,`document_id?`] |
| `memory.service.retain` | `text` (1–32768 bytes), `request_id` | delivery |
| `memory.service.inspect` | `document_id` | `document`, `evidence` |
| `memory.service.advanced` | `feature`, `action`, `body` (JSON ≤ 32768 bytes; any key named `bank`, `bank_id`, `namespace`, `tenant`, `tenant_id`, `account`, `account_id`, `user_id`, `uri`, `target_uri`, `table`, `schema`, `connection`, `connection_id`, `key`, `api_key`, `secret`, `sql` or `endpoint`, at any depth, answers `untrusted_selector`) | the backend's `data` |
| `memory.service.delete` | `document_id?` (absent clears the bank), `confirm: true`, `connection_revision`, `deletion_epoch` | `pending`, `operation_id`, `deletion_epoch` (or `reason: "remote_quiescence_unverified"`) |
| `memory.operations.list` | none | `operations` [delivery], `deletion` |
| `memory.operations.status`, `.retry`, `.cancel` | `id` | delivery |

A delivery is `{id, document_id, state, operation_id, error_code}` with `state` `queued`, `submitted`, `processing`, `completed`, `failed` or `delivery_unknown`; only `completed` means stored. A call that reaches the backend returns that backend response object (`evidence`, `document`, `operation`, `data`). `advanced` accepts only negotiated pairs: `bank_profile`/`bank_config` `get|update|reset`; `directives`/`mental_models` `list|create|get|update|delete|refresh`; `mental_model_history` `list|get|history`; `observations` `list|scopes`; `memory_edit` `edit`; `memory_invalidate` `invalidate`; `memory_restore` `restore`; `documents` `list|get|chunks`; `sessions` `create|get|messages|commit|delete`; `resources` `list|read|create|delete|ingest`; `tasks` `list|get|status|cancel|delete_record`. A turn allows eight calls, four of them retains (`turn_call_limit`). Codes include `confirmation_required`, `stale_confirmation`, `deletion_pending`, `unsupported_capability`, `operation_busy`, `operation_processing`, `uncertain_delivery_not_retryable`, `invalid_query`, `invalid_document`, `untrusted_selector`.

### Trusted Runner setup

These use one-use approvals: a `preview` stores up to 32 approvals bound to account, requesting Device, Runner, bot or profile, and revision; `apply` removes the approval before acting. Failure of an apply needs a new preview. Codes: `confirmation_required`, `approval_not_found`, `approval_expired`, `approval_stale`, `approval_action_mismatch`, `too_many_approvals`, `runner_required`, `runner_unknown`, `account_paused`, `runner_draining`, `setup_not_in_build`.

| Method | Params | Result |
| --- | --- | --- |
| `memory.embeddings.local.preview` | `runner_id`, `profile_id`, `profile_revision`, `plan` `{assets:[{kind (runtime|model|tokenizer), source (`{kind:"supplied",path}` or `{kind:"download",url}`), license, bytes, sha256}]}` | `preview_token`, `preview_digest`, `expires_in_seconds` (600), `runner_id`, `profile_id`, `profile_revision`, `total_bytes`, `assets` |
| `memory.embeddings.local.apply` | `runner_id`, `preview_token`, `preview_digest`, `confirm` | `installed`, `status` (`ready` or `runtime_unavailable`) |
| `memory.embeddings.local.status` | `runner_id`, `profile_id` | `status` (`ready`, `setup_required`, `runtime_unavailable`) |
| `memory.pgvector.initialize.preview` | `bot_id` | approval (below), `details` `{target,schema,sql,readiness}` |
| `memory.lance.binding.preview` | `bot_id`, `directory` (absolute), `create` | approval, `details` `{directory,create,table,namespace}` |
| `memory.lance.export.preview`, `memory.lance.import.preview` | `bot_id`, `path` (absolute; import file at most 8 MiB) | approval, `details` `{path,namespace,space,documents,sha256}` |
| `….apply` for the four bot actions | `bot_id`, `token`, `confirm` | `{initialized}`, `{status:"ready"}`, `{exported,bytes}` or `{imported}` |

A bot-setup approval is `{token, expires_at, action, runner_id, bot_id, profile_id, connection_revision, profile_revision, details}` and lives 300 s. `runner_id` on `embeddings.local.*` names the Runner and is removed before routing; the other setup methods use the bot's Runner. pgvector and Lance need the `memory-pgvector` feature, Lance both `memory-pgvector` and `memory-lance`. Asset downloads need an origin in the Runner's `BEANS_MEMORY_ASSET_ORIGINS`. Export writes plaintext vectors to a new 0600 file.

## Events

| Event | Data |
| --- | --- |
| `snapshot` | the `bootstrap` result: `roster.changed` fields plus each chat's newest 60 messages, relay state and turns in flight |
| `roster.changed` | `devices`, `bots`, `chats`, `routines`, `auto_review`, `paused`, `providers` (masked), `models`, `memory` |
| `memory.changed` | `bot_id?`, `status` |
| `message.added`, `message.updated` | `chat_id`, `message` |
| `message.removed` | `chat_id`, `message_id` |
| `chat.removed` | `chat_id` |
| `job.started`, `job.finished` | `chat_id`, `bot_id`, `job_id`, `routine_id?` |
| `job.retry` | `chat_id`, `bot_id`, `attempt`, `max_attempts`, `delay_ms`, `error` |
| `job.thinking` | `chat_id`, `bot_id` |
| `chat.usage` | `chat_id`, `usage` |
| `relay.status` | `connected`, `url?`, `update_required`, `error?` |
| `provider.auth` | `kind`, `url` |
| `plugin.auth` | `plugin_id`, `sign_in`, `url` |
| `plugin.auth.done` | `plugin_id`, `sign_in` |
| `pair.posted` | `nonce` |
| `pair.completed` | `nonce`, `device` |
| `identity.changed` | `has_identity` |

## Secret boundaries

Examples and logs use placeholders only. These methods carry a credential or plaintext account data across the socket:

- `identity.create` returns, and `identity.restore` takes, the recovery phrase.
- `providers.api_key` returns a saved key and base URL; `providers.connect_*` and `providers.list_models` take keys.
- `plugins.set_variables` takes plugin secrets. `mcp.get`, `mcp.list` and `mcp.save` return `mcp.json` entries with `env` and `headers`, which may hold bearer tokens.
- `bots.memory` returns plaintext bot memory; `files.path` returns a path whose decrypted bytes are on disk.
- `memory.*` replies mask secrets (`has_secret`); `memory.connections.set` and `memory.embeddings.set` take them as `replace` patches, and `memory.lance.export.apply` writes plaintext vectors.

`sync.account`, `providers.connect_*` replies and snapshots carry masked statuses only. Shared required-string checks and `chats.send` schema checks name fields without echoing values. Other errors are not guaranteed value-free: avatar checks can include supplied paths or names, and provider or relay failures can pass through response text. `provider.auth` and `plugin.auth` events carry OAuth authorization URLs, and every connected local client receives them.

## Sealed Runner verbs

A Device reaches another Runner through `kind=request` blobs ([Protocols](protocols.md#cli--relay)): the memory setup and `memory.service.*` / `memory.operations.*` verbs above, `memory.read`, `memory.write`, the `mcp.*` verbs except `mcp.parse`, `plugins.install`, `uninstall`, `variables`, `connect`, `sign_in.finish`, `sign_in.cancel`, `sign_out`, `detail`, `permission.answer`, `bash.stdin|stop|background`, `chats.send_now`, and `self_update.install` and `self_update.auto`, which fail closed. A sealed request waits 20 s (75 s for a plugin sign-in start, 150 s for `mcp.reconnect`, 330 s for memory setup). Only the memory setup verbs check that the requester is a known Device before acting; `runner_id` is a local routing field that the dispatcher removes before sealing.

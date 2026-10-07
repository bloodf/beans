# Memory services

The Runner's `memory_service` modules add account-configured memory beside the existing `MEMORY.md`, daily logs, local `recall`, and compaction memory flush. Configuration is shared by Devices; service operations execute on the bot's assigned Runner. See [Memory UI](memory-ui.md) for app helpers and [Identity](identity.md) for account encryption.

## Account configuration

`memory_config` is a separate schema-version-1 blob, encrypted with the account DEK and kind-associated data. Provider credentials have no memory fields. Connections include private secrets, typed backend options and optional embedding-profile references. Per-bot preferences contain binding, automatic-recall budget, future conversation/group capture consent and deletion epoch. Local filesystem/model/database paths remain private Runner bindings, not portable account config.

Records merge by logical counter, then exact Device id. A null value is a disconnect/reset tombstone. Unknown fields survive in their explicit flattened extension maps; known collection removals, including OpenViking identity keys, are not restored by unknown-field preservation. Equal-version conflicting known values fail closed. The private SQLite memory_config row stores account-encrypted bytes; config and relay ciphertext outbox change in one transaction with delivery invalidation and any deletion fence.

Protocol remains 4. Relay health advertises `memory_config_version: 1`; clients require it alongside protocol and roster floors before account sync. Missing or malformed capability evidence stops sync, rather than silently making settings local-only. Relay kind/slot `memory_config` carries opaque ciphertext independently of credentials. Every pull merges remote memory config before upload, and republishes locally newer records. Unsupported config fails without advancing that blob's application record.

Snapshots, `roster.changed`, and `memory.connections.list` contain explicitly selected masked fields: connection id/name/backend/revision and secret-presence flags; profile model/revision/dimensions and presence flags; known bot preferences. They contain no secrets, backend identity bindings, endpoint credentials or unknown extension fields. Secret edits use keep/replace/clear; omitted endpoint/options/profile reference preserves it, explicit nullable endpoint/profile clears it.

OpenViking binding edits are per-bot patches: omitted identities remain, null removes one, and whole-map replacement needs explicit confirmation. Cleared keys retain metadata but cannot initialize. Masked rows expose supported/blocked availability and sanitized reason, not endpoints or keys. Lance Cloud is blocked with lancedb_cloud_transport_unavailable before transport and preference activation; local Lance remains supported without fallback.

## Own-bot dispatch

Namespace is the full lowercase SHA-256 of `beans.memory.v1\0`, decoded canonical account Ed25519 public key, exact bot-id byte length and exact bot id. Renaming, provider/model changes and Runner assignment do not affect it. The model cannot choose account, bank, tenant, connection, URI, table, SQL or key. OpenViking stores typed namespace-bound USER-key or explicitly trusted-gateway identities; distinct bot namespaces cannot reuse authenticated account/user identity or USER keys across connections.

Namespaces are logical isolation, not server authorization. A Hindsight shared key can authorize more than one bank; hosted multi-user deployments need actual bank/tenant authorization at their server or gateway. Shell-enabled bots retain the Runner user's filesystem authority.

Core admission and dispatch recheck bot existence, assignment, account Pause, cancellation, connection revision, consent and deletion epoch. Immutable adapters are bound to scope and constructed explicitly, never through an inference-provider kind or generic backend factory. Saving configuration does not require a preinitialized adapter. Read-only health/setup negotiates an actual adapter; unavailable prerequisites report setup required rather than fabricated capabilities.

`MemoryBackend` executes health, retain, recall, inspection, supported deletion, operation status/cancel, native reflection and individually capability-gated advanced features. Hindsight profile semantics use supported config operations; directives, mental models/history, observations, curation and documents remain distinct capabilities. OpenViking sessions/resources/tasks do not pretend to be Hindsight reflection. pgvector and Lance use exact scoped vector/document operations and full embedding identity; neither natively extracts facts or reflects. No common-parity claim or paid synthesis fallback is made.

Advanced permission is exact feature-and-action membership, negotiated per route/verb and server flags. Config GET does not imply write/reset permission. Core filters document-delete actions out of advanced controls; deletion uses the confirmed revision/epoch-fenced path.

## Turns and local memory

A normal turn admits memory scope before turn work. Automatic recall uses task text, not an LLM query-generation call, once per turn. Hard limits are 5 seconds, 32 KiB response, 20 results and 8000 context characters; task queries are at most 4096 characters. Deduplicated evidence is labelled untrusted historical data, not instructions, in ephemeral TurnNotes after transcript cache points. Outage leaves existing local memory usable and emits visible degraded memory status.

Bot-bound service tools accept only text or query. Native reflect is registered only when negotiated. The per-turn limit is eight calls and four retains including failed attempts; this is a request bound, not a dollar-spend ceiling. Housekeeping's whitelist remains local memory_update/memory_log and never calls remote memory. `memory.read`/`memory.write` and existing local tools stay independent.

## Durable capture and deletion

Conversation capture is off by default and snapshots consent at turn admission. Completion rechecks the same consent/connection versions and epoch. Only that turn's visible completed user/own-assistant text with chat/message/speaker provenance is captured. Group text needs separate opt-in. History, attachment bytes, tool arguments/results, shell output, permission payloads and hidden thinking are not captured. Credential scrubbing is best-effort, not DLP; sanitized payload size is checked again after redaction expansion.

Private memory_turn_admissions receipts persist both enabled and disabled admission, exact job/bot/chat/trigger identity and sanitized frozen trigger text before turn work. Replay cannot adopt newer consent or edited source text. Captured intent and its consumed receipt marker commit atomically; completed capture is not backfilled after restart.

A dedicated SQLite memory_deliveries queue atomically stores sanitized frozen payload, sources/content hash, namespace, revisions and intent. Full domain-separated document/request hashes provide stable retry identity, not exactly-once billing. Queued intent is claimed by exact-row CAS; submitted is committed before traffic. Restart turns submitted into delivery_unknown. Async processing keeps the service operation id. Failed or uncertain retains are not automatically billed again without verified backend idempotency. Admission/call bounds are checked before marking unsent work submitted.

Opted-in capture can deliver at admitted-turn completion within its cap. User RPCs explicitly retry/poll/cancel. Unattended worker enablement requires enforceable downstream-spending policy; without that evidence it is rejected. There is no unbounded remote housekeeping/retry worker.

Deletion confirmation binds exact bot/document-or-bank, connection revision and epoch. A transaction increments the epoch, stores a pending fence and invalidates queued writes before deletion. Old in-flight/unknown/processing operations keep cleanup pending. The quiescence barrier queries every scoped uncertain row, not an operation-list page. Fences win late delivery responses; old operation ids cannot be polled against a replacement connection. No verified quiescence means pending cleanup, not guaranteed erasure. Disable/disconnect/unpair/bot deletion does not silently delete a remote bank. Provider backups and Lance history can retain data.

## Trusted Runner setup

Setup RPCs route to the assigned Runner; local embedding setup explicitly selects a Runner. One-use memory-only approvals bind account, requesting Device, Runner, exact configuration revision/epoch, target and action; expiry/restart/unpair or stale values require a new preview. Apply accepts affirmative confirmation and the frozen token, not new SQL or targets. Backend and installer approvals add their own exact target/plan validation.

pgvector initialization previews actual server identity, trusted schema, non-destructive SQL, permissions and readiness before applying an approved transaction. Lance local binding previews exact directory, table creation permission and namespace; export/import freezes bounded provenance/vector data with exact namespace/fingerprint and source/destination. Private SQLite memory_runner_bindings stores Runner-local paths and revisions. Cloud region/key configuration remains portable but inactive under the transport block. Moving a local database requires explicit validated transfer, not account-wide path synchronization.

A future live synthetic test needs separate exact action/data/endpoint approval and a concretely enforced US$5 total ceiling covering extraction, embedding, reranking, reflection, retries, consolidation and refresh. US denotes currency, not geography. Request/token bounds and post-hoc billing are not dollar enforcement; unknown total-cost enforcement means fixtures only.

## Embeddings

`crates/cli/src/embeddings` is separate from inference providers. Portable profiles explicitly select `api` or `local_cpu` and pin model, revision, exact dimensions, `none`/`l2` normalization, cosine/dot/Euclidean distance and exact document/query prefixes. Local profiles also pin ONNX/tokenizer SHA-256, maximum tokens, padding/special-token settings, exact tensor names and mean/CLS/already-pooled output handling. Paths and runtime availability belong to private Runner bindings.

The full domain-separated SHA-256 vector-space fingerprint covers canonical semantic metadata, model/revision, preprocessing and engine versions, dimensions, normalization/distance, both prefixes, endpoint or local asset identity, and unknown semantic extensions. Credentials and Runner paths do not participate. Every batch retains that fingerprint and input ordering; count, finite components, exact dimensions and nonzero norm are validated, with configured L2 normalization computed using an f64 norm. Storage checks exact fingerprints and monotonic approved index generations. A changed space needs a complete replacement index and storage-owned validated atomic switch; the prior generation remains available for rollback.

### Compatible API

The Runner posts to the exact trusted URL ending in `/embeddings`, with the configured model, dimensions, float encoding and purpose-specific prefixed inputs. It uses optional sensitive Bearer authorization, shared `lorca-tls` certificate trust and system/environment proxies. HTTPS is the default; insecure HTTP needs explicit approval. Redirects are disabled. Request sizes, batch counts, decoded response bytes and elapsed time are bounded, including chunked bodies; cancellation drops the request.

Responses must name the pinned model and provide each batch index exactly once, in range, with the exact vector count and shape. Generic compatible responses do not attest a model revision: the endpoint operator must provide a stable immutable model/revision mapping. Errors expose only sanitized categories, not credentials, request text or service bodies. No discovery inference, retries, startup traffic or paid/backend fallback runs implicitly. API use sends plaintext embedding input to that configured endpoint, outside the relay's zero-knowledge boundary.

### Local CPU and approved assets

`embedding-local` implies `runner` and enables the official `ort = 2.0.0-rc.13` wrapper with `std`, `load-dynamic`, `api-27`, plus the compatible stable `tokenizers = 0.22.2` API with `onig`. Default features are disabled; binary/model downloads, model hubs and GPU providers are not enabled. Phone dependency graphs exclude these SDKs. Native execution loads an explicitly approved compatible dynamic ONNX Runtime, selects CPU, disables telemetry and keeps native diagnostics out of public errors.

An explicit local-setup preview freezes exactly runtime/model/tokenizer sources, licenses, byte counts and full checksums. Sources are supplied absolute files or explicit download URLs restricted to administrator-approved exact origins. Preview performs no acquisition. Apply requires the same unexpired single-use token/digest and affirmative user consent, plus core's account/Device/Runner/profile-revision fences. Downloads use verified TLS/proxies, no redirects/retries or archive extraction, and streamed exact size/hash checks. Supplied files are copied and checked too. Fresh private create-new staging files are flushed before atomic same-filesystem directory installation; failures and cancellation clean only that attempt's staging. Existing assets are not overwritten, and no acquisition runs at startup or as fallback.

Before native initialization, runtime bytes are copied from one opened source handle into a fresh private read-only file/directory and checked again. ORT opens that protected copy, not a subsequently mutable source path or replaced symlink. Its process-global binding retains the exact copy; a different runtime binding requires restart. This prevents ordinary source replacement, not an adversarial same-user filesystem sandbox. Model/tokenizer parsing uses verified in-memory bytes. The actual tokenizer applies explicit right truncation and batch-longest right padding; masked pooling follows the configured output contract rather than guessing tensor names or dimensions.

Cancellation fences native setup before initialization. Once non-preemptible environment/session creation starts, the Runner awaits its completion before returning cancellation or timeout, which can exceed the requested deadline. Inference uses per-run ORT termination on cancellation/drop and retains the session lock until native work finishes. Missing assets, incompatible runtime or mismatched model/tokenizer contracts report setup/runtime/contract errors, never synthetic vectors.

Source compilation and offline fixtures exercise the real SDK interfaces, tokenizer, transport and setup mechanisms. Actual native inference and signed clean-host Mac/Linux/Windows runtime packaging require approved real files and platform checks; they are not established by those fixtures.

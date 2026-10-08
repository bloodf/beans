# Memory service UI

The memory-service client boundary consists of isolated TypeScript and Foundation models:

- `desktop/src/model/memoryService.ts` provides the framework-independent request adapter, editable per-bot draft and masked reply readers.
- `macos/Sources/Beans/Model/MemoryService.swift` provides Foundation-only request, draft, secret/options and masked reply types.
- `desktop/src/model/memorySetup.ts` and `macos/Sources/Beans/Model/MemorySetup.swift` provide exact typed local-asset, pgvector and Lance preview/apply boundaries and ephemeral confirmation drafts.
- `desktop/native_memory.go` evaluates the existing TypeScript service/setup models through `assets/memory.js`, built from `src/model/nativeMemory.ts`. Go handles expose the same consent drafts, masked readers, secret/options request constructor, deletion fence and one-use setup approvals; connection/account transitions invalidate handles. Native RPC dispatch uses the existing CLI connection, exact memory allowlist and demo refusal. The native connections panel displays only validated masked configuration and does not automatically check remote Health. Full native editing/setup sheets remain cutover gates; the working Solid sheets remain default.

The native memory panel opens interactive connection and per-bot forms (`native_memory_forms.go`, `native_memory_actions.go`). Connection controls distinguish keep/replace/clear endpoints and secrets and typed backend options, including explicit OpenViking bot binding patches. Bot controls use the shared draft for exact plaintext/group consent, bounded recall and capture values. Reload refreshes authoritative deletion status without rebasing unsaved consent; only a successful save followed by validated reload rebases it. Remote Health is explicit. Document/bank deletion requires confirmation and negotiated capability, admits the shared deletion lock before dispatch, and refreshes authoritative preferences after either result. Concrete pgvector/Lance/local-asset setup displays the preview, requires confirmation, and consumes a target/revision/expiry-bound one-use approval before apply; failed apply requires a new preview. Pending mutations disable editing and cancel; failures retain drafts. Native forms participate in close/quit ownership guards. A reconnect invalidates pending panel state and approvals. The embedded setup runtime supplies a non-network URL parser preserving HTTPS, userinfo and fragment rejection.

Native setup presentation reads the shared approval object's sanitized immutable `preview`, never the raw RPC reply. Operation metadata remains visible before Health; Status and Cancel controls and dispatch require their separate negotiated `operation_status` / `cancel_operation` flags. Connection transitions retain local form values—including unsaved replacement secrets—under the admitted account ID while clearing RPC results, pending state, consent/deletion handles and setup approvals. A matching authoritative account snapshot restores the form; bot recovery reloads authoritative preferences into fresh handles without replacing user choices, and requires renewed plaintext/group approval. Other-account forms remain hidden and participate in quit ownership; explicit confirmed discard is available. Retention is in process, not a durable secret-storage promise.

Each adapter accepts the application's request transport rather than opening a second connection. Desktop `AppStore.memoryRequest` forwards only the exact common and concrete setup RPC allowlist to the existing private transport; it rejects arbitrary methods, superseded setup aliases and demo mode. General's account Memory action calls `presentMemoryConnections`; the desktop local memory sheet offers `presentBotMemoryService` without changing local editing or hash-conflict handling. Account and bot sheets use typed controls for connection patches, preferences, negotiated service actions and exact setup previews.

Desktop memory sheets keep their header and footer outside the independently scrolling `.sheet-content` body. Class-scoped rules prevent flex shrink and visible body overflow from placing fields beneath footer hit targets; maximum body scroll and keyboard traversal leave confirmation actions visible.

## Account connection edits

Connection secrets use explicit keep, replace and clear patches. Keep never resends a stored key, replacement preserves the exact supplied bytes, and clear is distinct from an empty replacement. Replacement is bounded to 8,192 UTF-8 bytes. Swift description and debug output redact replacement values.

Omitted endpoint, embedding profile and backend options preserve the saved values. Explicit null clears endpoint or embedding profile. Backend options have typed Hindsight, pgvector, LanceDB and OpenViking variants; their tag must match the connection's backend. OpenViking binding edits use exact bot IDs and their own secret patches. Core computes the private account/bot namespace. Form data cannot supply a namespace, bank, tenant, arbitrary backend JSON or raw OpenViking `api_key`.

Read models retain only masked connection, embedding-profile and preference fields. Unexpected endpoint, secret, backend-option and private payload fields are discarded. Unknown backend or schema versions fail decoding rather than selecting a substitute backend.

Masked connections include required `availability: supported|blocked` and `reason: null|string`. Supported means eligible configuration, not initialized or ready. LanceDB Cloud connections are blocked with `lancedb_cloud_transport_unavailable`; Runner-local Lance remains supported. The desktop displays the blocked reason, excludes blocked connections from new bot selections, prevents activation saves and hides their setup actions. Core also rejects Cloud activation and initialize/recheck before Cloud calls. No endpoint, path or key is exposed by this availability metadata.

## Per-bot consent drafts

New drafts leave auto-recall, conversation capture, group capture and unattended capture off. Enabling remote query or conversation plaintext requires approval for the bot, connection and exact capture choices. Group capture requires separate approval and conversation capture. Changing the connection clears approvals, including a change back to the prior connection. Increasing the delivery cap requires renewed plaintext approval; editing an existing approved preference without extending consent preserves it.

Capture requests describe future admitted-turn capture only. They contain no historical backfill or client-selected namespace. Unattended capture is refused with `enforced_spending_policy_required` while enforceable service-side total-cost policy is unavailable. US$ names currency, not a geographic restriction.

Recall budgets validate the core bounds: 1–5,000 ms, 1–32,768 response bytes, 1–20 results and 100–8,000 context characters. Capture delivery caps are 0–4 per turn. Invalid values fail before dispatch instead of silently expanding or clamping approved behavior.

## Capabilities and status

Controls consume negotiated capabilities, not a backend-name matrix. Advanced actions require both the feature and its exact verb in `advanced_actions`; absent or empty action lists hide them. Vector document editing uses `memory_edit` / `edit` with only `document_id` and `text`; advanced document deletion is unavailable. Health preserves `ready`, `degraded`, `setup_required`, the optional reason and the deletion-pending flag. Opening the bot sheet reads local configuration and operation metadata, not remote Health. Service checks are explicit; action refreshes update an already checked status.

Operation state remains queued, submitted, processing, completed, failed or delivery_unknown. Only completed means stored. Pending deletion is not verified erasure. The helpers retain core-issued operation IDs and deletion epochs without issuing automatic retries, retain, reflect or deletion requests.

Authoritative preferences and the editable draft have separate ownership. Service action/status refresh and Reload preserve unsaved choices; only a successful preference save followed by validated reload rebases the draft. Every admitted deletion fence locks document deletion and bank clearing before dispatch. Both successful and failed attempts require authoritative preference refresh; failure or an epoch below the returned deletion epoch keeps the lock. Refreshing authoritative status advances the saved epoch before permitting another deletion without discarding the draft.

## Concrete setup drafts

Local embedding previews use the selected Runner, profile ID, exact profile revision and an AssetPlan containing runtime, model and tokenizer. Every artifact carries its supplied path or exact HTTPS source, license, byte count and full SHA-256. The preview returns its opaque token and digest, ten-minute lifetime and approved assets. Apply sends only Runner, token, digest and affirmative confirmation. Installed-but-runtime-unavailable is distinct from ready; no inference or fallback is implied.

Pgvector preview retains the actual connected database/role/server/session target, schema and non-destructive SQL. Lance binding preview retains the exact Runner directory, table and trusted core namespace. Export/import preview retains the exact plaintext source/destination, namespace, document count, checksum and full vector-space fingerprint/generation. Dimensions alone do not establish migration compatibility.

Bot setup uses the action-specific preview method, five-minute expiry and a one-use bot/Runner/connection-revision token. Its apply request contains only bot ID, token and confirmation, never fresh paths or SQL. Drafts reject missing confirmation, stale target/revision and expiry before dispatch, and consume local approval before a request is sent; even a failed apply requires a fresh preview. Core remains the authority for identity, deletion epoch, runtime approval, one-use tokens and exact setup effects.

## Local editor and verification

The service boundary is independent of local MEMORY.md. `macos/Sources/Beans/Sheets/MemoryViewController.swift` continues to use its existing hash-based conflict protection; the desktop local editor is unchanged.

`bun test desktop/src/model/memoryService.test.ts desktop/src/model/memorySetup.test.ts` exercises consent transitions, secret/options patches, bounded requests, masked status and exact setup approvals with fixture transports. `macos/Tests/MemoryServiceDraft.swift` and `macos/Tests/MemorySetupDraft.swift` are isolated Foundation executables compiled with their memory model sources, without a full application build. Fixture verification performs no remote retain, reflect, asset acquisition or DDL.

`desktop/src/ui/sheets/memoryLayout.browser-check.js` checks mounted production sheets in an offline browser fixture. After scrolling or focusing a target, `assertMemorySheetLayout(fieldLabels, actionLabels)` verifies viewport containment and center-point hit ownership for controls and visible header/footer regions. The fixture checks last fields at maximum body scroll and keyboard access to footer actions without sending service requests.

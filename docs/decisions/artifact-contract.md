# Artifact identity, storage and sync contract

Decision for issue #92. This is a discovery proposal for downstream implementation. The scenarios below are design analysis, not runtime observations or acceptance evidence.

## Source boundary

The source baseline is `origin/main` at `1a1386a15696a50eddff915036f75cde2675203b`, reconciled into PR #354. The issue's protocol-4 wording predates the current protocol-5/`beans-v2` boundary. This proposal does not restore protocol 4.

Current mechanisms and exact source anchors:

- [Protocols](../architecture/protocols.md#cli--relay) defines binary encrypted `file` transport, latest-only slots, chat groups and deleted-group rejection. `crates/cli/src/files.rs::{store_many,push_blob,ensure_local}` stages local attachments, encrypts with account DEK and `file` AAD, and fetches/decrypts files. Local Device caches contain plaintext. Transport IDs are not content integrity proofs.
- [Tools](../architecture/tools.md) describes reviewed proposals. `crates/cli/src/plugins/mcp.rs::{propose,save_proposal}` binds approval to the exact draft, rechecks Pause, Runner assignment and write capability, then publishes only to an absent target. Unix uses anchored directory handles and no-replace publication; `proposal_windows::save_proposal` supplies the Windows boundary. An allowed card replay can return success without writing again. It does not prove that an artifact was indexed or uploaded.
- `crates/cli/src/artifacts.rs::{ArtifactRevision,ArtifactSave,proposal_id,pin_id,acceptance_fingerprint}` already implements pure validation and canonical identities. `artifacts/store.rs::{reserve_workspace_tx,commit_artifact_acceptance_tx}` contains private borrowed-transaction persistence. [Tools](../architecture/tools.md) records their limits: production schema installation, API admission, workspace effects and sync are not activated by those units. Their existence is not end-to-end artifact delivery.
- [Relay](../architecture/relay.md) describes the file object store, quota and cleanup. `crates/relay/src/routes.rs::{put_blob,put_file}` admits only `db::KINDS`, prechecks ciphertext quota, and writes binary file objects before their rows. `crates/cli/src/sync.rs::{POLL_KINDS,NOT_CHAT_KINDS}` does not poll artifact metadata. Unknown kinds are not currently a negotiated artifact feature.
- [Tasks](../architecture/tasks.md) defines account incarnation, execution ownership, replay denial and immutable unknown-effect receipts. These fences remain independent requirements.

## Decision and alternatives

Choose immutable account-encrypted `file` blobs for revision bytes, plus a new encrypted append-only `artifact` metadata kind in the existing blob log. This reuses the existing file store, bearer authentication, account key and quota. It requires new relay kind admission, client polling and capability negotiation in dependent work. No second object store, dependency or paid service is required. No artifact names, content hashes, paths or contents appear in relay plaintext metadata.

Runner-local files with on-demand remote fetch require Runner availability when a phone opens the result. Reject that option for durable cross-Device viewing. A latest-only roster field loses revision history and couples artifact updates to roster CAS; reject it. Content-only identity merges separately approved outputs with equal bytes; use provenance identity instead. Hashes verify bytes, not authority.

Artifacts remain chat-owned in version 1. Deleting their source chat removes them. A cross-chat library may reference an artifact while its source chat exists; a reference confers no independent lifetime. Detaching an artifact from chat deletion requires a separate future contract. This choice agrees with existing group cleanup and avoids silently changing retention.

## Exact record and identity handoff

Use the already validated `ArtifactRevision` shape, with all fields required, including explicit nulls. Unknown or duplicate fields reject in version 1.

| Field | Proposed meaning |
| --- | --- |
| `version` | Integer `1` |
| `id` | Stable `art-` plus 48 lowercase hex characters |
| `revision_id` | Canonical hyphenated UUID identifying one immutable revision |
| `name`, `mime` | Nonempty bounded display metadata, MIME containing `/` |
| `size` | Plaintext byte count, at most 100 MiB |
| `content_hash` | Lowercase 64-hex SHA-256 of exact plaintext bytes, without text normalization |
| `parent_revision`, `parent_content_hash` | Both null for a root, otherwise the exact immutable parent UUID and byte hash |
| `file_id` | Exactly `<revision_id>.file` |
| `chat_id`, `runner_id` | Source chat and assigned Runner identity |
| `bot_id` | Origin bot, or null for a user-originated record |
| `card_id`, `attachment_id` | Proposal card or pinned attachment provenance, otherwise null |
| `created_at` | Finite numeric Unix seconds; display metadata, never conflict ordering |
| `path` | Bounded workspace-relative path or null; never a relay object path |

Identity uses the existing domain-separated algorithm: SHA-256 over UTF-8 domain, one zero byte, then each UTF-8 component preceded by its unsigned 64-bit big-endian byte length. Take the first 24 digest bytes as 48 lowercase hex characters. Proposal domain is `beans.artifact.proposal.v1`, components `chat_id, bot_id, card_id`. Pin domain is `beans.artifact.pin.v1`, components `chat_id, attachment_id`. Prefix with `art-`. Account storage namespaces isolate identical derived IDs across accounts. These identity hashes are domain-separated; `content_hash` deliberately retains the existing raw SHA-256 byte verifier.

A child preserves `id`, `chat_id` and `runner_id` and names an available verified parent. Missing parents mean incomplete state, never permission to create a new root. Equal bytes in two roots remain separate artifacts. Concurrent children remain immutable branches; no timestamp winner may erase a branch.

Revision requests use exactly `ArtifactSave { id, expected_revision, expected_hash, content, path, request_id }`, with explicit nullable `path`. Text limits remain 20,000 Unicode characters and 65,536 bytes. Both expected values must match the authoritative parent. Mismatch returns `artifact_conflict` with `current_revision` and `current_hash`, without workspace or outbox mutation. Results retain `id, revision_id, content_hash, workspace, sync`; these are separate effect states, not a generic saved flag.

## Publication and authorization

The first proposal publication still requires an absent workspace target. Never make `save_proposal` overwrite to support revisions. A revision of an existing destination needs a new reviewed compare-and-replace mechanism: capture parent bytes and expected hash, anchor the destination, reject aliases and changed targets, recheck live account/Runner/task authority at commit, and journal the exact attempted effect. Until that mechanism exists, revision replacement is unavailable. Publishing to a new absent path is separately reviewable; approval of the old path does not authorize it.

Admission must bind the account incarnation, requester membership, Runner, owner epoch, account epoch and exact request body. Use existing signed sealed request admission for remote Device requests; ciphertext alone is not authenticated approval. Preserve Pause, bot write capability, draft binding, cancellation and revoked-requester checks. A phone can request or review; it never becomes a Runner.

Persist the immutable revision snapshot, exact encrypted payloads and acceptance intent before a workspace attempt. Persist observed workspace outcome and ordered upload intent transactionally. Reuse `acceptance_fingerprint` for the private request commitment, with account/epoch authority fences checked separately. Exact retries recover their stored result; changed bodies under one request ID fail `artifact_request_body_changed`. Never infer a successful write from an allowed proposal card alone. An uncertain attempt remains `artifact_workspace_uncertain` and needs explicit reconciliation, not automatic replay.

Local task-epoch namespaces and imported account namespaces remain distinct. Importing revisions cannot mint local task leases. Account removal, task deletion, execution-owner changes and tombstones must fence late callbacks and retries. Preserve immutable started/unknown receipts and their replay denial even after explicit resolution. This proposal neither shortens retention nor authorizes cleanup of those records.

## Sync and capability gate

Propose health capability `artifact_version: 1` under protocol 5 and format `beans-v2`, plus version-1 support advertised by Device cores and local `hello`. These are new fields, not existing capabilities. Writers require exact supported capability evidence at relay and participating cores before enabling artifact mutation. Missing, malformed or unsupported evidence blocks artifact publication without dropping local intent. Existing non-artifact operations keep their existing gates.

Use `kind=file`, ID `file_id`, `group=chat_id` for encrypted bytes with `file` AAD. Use `kind=artifact`, ID `<revision_id>.record`, `group=chat_id`, no slot and no recipient for encrypted revision metadata with `artifact` AAD. `revision_id` remains the immutable logical UUID; the `.record` suffix identifies its metadata transport blob, matching the private store upload intent and cleanup ID. Upload bytes before metadata and retain exact retry ciphertext. A persisted byte acknowledgment gates metadata eligibility. Both count against the existing ciphertext quota; size limits include encryption overhead. Quota refusal leaves the revision pending/blocked and never reports synced. An interrupted upload may leave an unreferenced object; cleanup follows existing relay rules, with no new retention policy here.

A receiving core validates/decrypts metadata, its parent and the file size/hash before exposing an openable revision. Metadata arriving without bytes remains incomplete and retryable. Do not reinterpret `file_id` as an attachment ID accepted by `files::store_many`; its attachment ID validator is a different boundary. New artifact cache/reader integration must use validated revision IDs and preserve private local files.

An incapable upgraded client renders "Update Beans to view this artifact" and offers no open/edit/delete action. Existing clients cannot be assumed to render that message: the current polling list excludes `artifact`. Therefore dependent work must supply the fallback through a compatible chat representation and capability-aware UI, while preserving unknown metadata without interpreting it. Do not send new metadata to an old parser or claim that unknown-kind ignoring already provides safe artifact rendering. Unknown future versions remain opaque and cannot trigger effects or be rewritten as version 1.

## Deletion and replay

Artifact deletion uses the existing parsed shape `{ version: 1, op: "remove", id, chat_id }`. Chat removal uses `{ version: 1, op: "remove_chat", chat_id }`. The latter's stable blob ID follows `removed_chat_id`: domain `beans.artifact.chat-removal.v1`, component `chat_id`, prefix `ach-`, suffix `.removed`.

Propose ungrouped, unslotted encrypted `artifact` tombstone envelopes so group deletion cannot erase the replay fence itself. Individual removal IDs must be durable unique operation IDs. Apply tombstones before admitting late revision intents; remove revisions and byte objects through existing blob deletion queues. For chat removal, persist the local fence before `DELETE /v1/groups/{chat_id}`, preserving the relay's `409` deleted-group defense and local outbox cancellation. Keep account-lifetime local deletion fences; the existing 180-day relay deleted-group sweep is not sufficient evidence to discard them. Reconnection and history replay must carry tombstones before any retained revision publication. Tombstones do not authorize a removed Device or a stale task to perform effects.

Artifact removal does not delete the chat. Chat deletion deletes its artifacts, including pinned copies and revisions, while retaining replay-denial evidence. Workspace files already published are user files; chat or artifact deletion does not silently remove or overwrite them. Account deletion retains its existing explicit authority and relay boundary. Retention implementation remains owned by its separate issue/PR.

## Design analysis: approve, open, delete

These are paper scenarios on 2026-11-07, against the source baseline above. No app, phone, relay or real account was exercised.

1. Approve on Runner A. With live authority and an absent target, approval publishes exact reviewed bytes. The proposed artifact adapter derives the provenance ID, reserves its snapshot, observes publication and queues encrypted bytes before metadata. Existing targets fail without overwrite. Crash uncertainty retains its receipt; an allowed card does not substitute for a publication receipt.
2. Open on phone B. After supported capability negotiation, B imports metadata, resolves the parent and downloads/decrypts/verifies bytes. A can be offline. Missing bytes show incomplete state; unsupported clients show the update fallback once that adapter exists. Source today does not demonstrate this artifact flow on a phone.
3. Delete the chat. The deleting Device persists the removal fence, cancels queued publication and deletes the relay group. B removes the artifact view when deletion syncs; a late A cannot resurrect it. The tombstone remains outside the removed group, and published workspace files stay intact. These ordering requirements still need integrated implementation and tests.

## Dependent work handoff

Downstream storage/schema work consumes the exact record, save/result shapes, namespace separation, snapshot commitment and tombstone rules above. Sync work consumes `artifact_version`, kind/AAD `artifact`, byte-first acknowledgment, grouped revisions and ungrouped deletion envelopes. Revision editing work consumes the expected revision/hash compare rule and must supply the new workspace mechanism. Client work consumes verified/incomplete/unsupported states and the compatible fallback. No consumer may infer runtime readiness from this document or the isolated parser/store units.

Editorial updates to dependent issues should link this decision and copy these exact names while preserving phase, labels, explicit dependencies and managed roadmap links. The parent receives a reviewable additive patch; no issue metadata or acceptance boxes are changed here.

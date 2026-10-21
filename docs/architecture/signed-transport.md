# Signed Device transport

## Envelopes, custody and dispatch

Addressed `job`, `job_cancel`, `job_result`, `request` and `response` deliveries use a closed version-1 signed envelope inside the existing sealed box. The signature covers `beans/envelope/v1\0` and exact bytes naming version, account identity, sender, recipient, kind, UUID, integer creation time and body. Jobs and Requests bind `requested_by` to the verified sender. Replies bind to the Runner captured when sent, not later bot assignment. Both existing inner Request signature domains remain unchanged.

Encrypted Device metadata advertises `envelope_version: 1`. Unsupported or absent capability preserves Device visibility but refuses addressed production and upload, without fallback. Protocol 5, account format, encryption and relay health remain unchanged. Old queued addressed ciphertext without the durable producer marker remains quarantined, never re-signed or automatically replayed.

Inbound custody, replay identity and recoverable delivery intent commit before dispatch or forward cursor publication. Missing sender metadata retains deferred custody and retries validation after authoritative membership refresh. Legacy and rejected deliveries retain distinct dispositions and are not automatically deleted. Every Request and incoming Job claims its operation before effects; interrupted claims become uncertain rather than executing again. Exact retries preserve bytes and cannot reset completed or uncertain operations. Completed Request retries reuse saved signed response bytes.

Session entry and machine-list invalidation refresh authoritative membership before pulling or dispatching. Pending intent binds the account identity, recipient and sender box key. Membership publication, claim and execution admission share the existing local lifecycle boundary without holding it across effects or awaits; queued execution captures the process-local incarnation, and account teardown clears durable transport intent. Held control scans durably activate staged Stop, completion and continuation intent without advancing the forward cursor. Identity conflicts retain the original binding and one payload-free digest record per conflicting relay id; unrelated page items continue, while storage failures roll back admission.

For local account-owned atomic persistence, lifecycle -> roster -> state may remain held through SQLite commit. These guards release before notifications, event construction/publication, callbacks and network awaits. This local SQLite exception does not authorize holding transport admission across external effects.

Pages containing only account blobs do not require addressed-transport identity. The pending transport pump does no work when the local machine identity is absent; signed addressed admission still requires it.

Forward and first-sync pages consult durable rejection before new-work admission. A rejected conflicting delivery advances through its rejection path during an update lease; genuine pending retries still require admission. Failure reading or persisting that disposition stops the page.

Authenticated Stop persists a job-id tombstone serialized against execution claim. Stop before arrival prevents execution; later Stop reaches the registered cancellation token, including the claim-to-registration gap. Already-issued effects cannot be undone. Requests/responses expire after one hour, other addressed envelopes after eight days, with one hour of future skew. Replay identities and cancellation tombstones remain until account teardown.

## Local status and refused outbound intent

Snapshots include local `transport` status: bounded persisted incompatible/deferred/rejected/uncertain items and `legacy_outbound_count`. Status omits ciphertext, signatures and payload contents. Update incompatible peers, inspect the originating chat and effects, then explicitly request new work if needed. Reconnection and updates never release legacy or uncertain intent automatically.

Relay rejection of signed addressed intent records `transport_relay_rejected` with its HTTP status. The original id, recipient, ciphertext and version marker remain retained across restart; rejected rows are skipped so account uploads continue. Identical requeue does not release quarantine, and changed bytes under the same signed id conflict. New explicit work uses a new identity.

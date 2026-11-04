# Portable setup contract — discovery outcome for #220

This is a configuration recipe, not an account backup, content export, live-sync mechanism, Runner clone, or second identity. Existing-account setup uses Pair first: protocol-5/`beans-v2` pairing preserves existing account sync semantics. Importing the recipe into another account creates configuration only; it never carries identity, permissions, credentials, content, execution state, or machine-local authority.

## Ownership and inventory

| Data | Owner today | Recipe policy | Import effect |
|---|---|---|---|
| Account identity, master secret, device/machine keys, account DEK, relay bearer | Identity device / each paired Device | Exclude | Target account remains selected; never pair or register implicitly |
| Bots: name, description/instructions, provider/model/thinking identifiers, `look`, legacy symbol/accent | Encrypted account roster; appearance contract in `avatars.md` | Include only these profile fields, bounded and schema-versioned | Create new bot IDs; never copy source IDs or overwrite existing profiles silently |
| Routines and saved workflow definitions | Encrypted account roster (`bots.md`) | Include definitions only; omit run state, checks, logs, timestamps, outcomes | Create routines with `is_enabled: false`; save workflows as non-executable `WorkflowRecord` definitions, as detailed below |
| Capabilities / grants, Auto-review, account Pause, policy receipts | Encrypted roster/policy | Exclude | New bots receive least capability defaults; no imported grant or approval |
| Provider IDs/endpoints and model IDs | Bot profile plus account credentials (`providers.md`) | Include nonsecret connection hints only; never keys, OAuth, refresh tokens | Treat as unconfigured; validate syntax locally during import; a separate confirmed reconnect on the selected Runner validates destination reachability |
| Plugins, manifests, OAuth/API credentials, variables, installed state, catalog/tool cache | Runner-local `plugins/` | Include only stable marketplace plugin IDs and optional version/pin hints; no custom executable manifests or variables | Show preview; explicit install from trusted current marketplace, then separate setup/sign-in and bot grant |
| Skills | Runner-local managed library (`skills.md`) | Exclude bytes and source paths; optional opaque reference is informational only | No copy or activation |
| Memory service connections/preferences | Account-encrypted `memory_config`; paths and bindings Runner-local | Exclude; profile contract #107 governs future profiles | No service binding or consent copied |
| Bot `MEMORY.md`, topic notes, diary, chats, attachments | Runner-local memory or account content | Exclude; content export is #150 | No content import |
| Local paths, filesystem roots, embedding assets, MCP server config, machine metadata | Runner-local | Exclude | User remaps and configures locally |
| VM disks/controllers, jobs, approvals, pending requests, routine checks/runs, push tokens, relay queues/caches | Device/Runner/relay ephemeral or local | Exclude | Never cloned or resumed |

Definitions are bounded inert data. Import must not run hooks, scripts, routine checks, plugin servers, workflows, or model calls. Marketplace references do not imply trust or installation. Provider endpoints are untrusted hints requiring validation and explicit confirmation; secrets are always re-entered or connected through existing auth flows.

Imported routines persist `is_enabled: false`, `paused_reason: None`, `check: None`, `last_run_at: None` and `last_outcome: None`; their creation and schedule-anchor timestamps are newly assigned on the target. There is no invented `paused_reason: "imported"` value. Only a separate explicit user enable action may set `is_enabled: true` and reset the schedule anchor. Import does not call the ordinary create-and-check path or Run Now.

Imported workflows persist only the existing `WorkflowRecord { id, bot_id, name, definition, created_at }`, with fresh IDs and target timestamps. That record has no enabled/status field and the current workflow implementation has no execution path: saving it only makes the definition available for listing. No job, routine or execution trigger is created from it. Any future workflow executor must treat these existing definitions as disabled until an explicit user enable action; adding an executor or reopening Beans must never activate an imported definition. Imports also obey the existing 20-workflows-per-bot and 256-total roster limits before staging, in addition to the file limits below.

## Endpoint hint validation

Import validation is local syntax and policy validation only. Accept an absolute `https` URL with a valid DNS hostname or canonical IP literal and a port in 1–65535; reject credentials, query strings, fragments, malformed/ambiguous hosts, `localhost` and its subdomains, and non-public address literals, including unspecified, loopback, private, link-local/metadata, multicast, IPv6 unique-local and IPv4-mapped equivalents. Relative URLs and other schemes, including `http`, `file` and `javascript`, are rejected. This stricter recipe policy does not change manually configured local providers.

Parse, preview and commit perform no DNS lookup, TCP connection, HTTP request or other network probe. Passing syntax checks does not establish that a hostname resolves publicly or that the provider is trusted. After import, reconnect is a separate explicit user action confirming the endpoint on the chosen Runner. It revalidates the URL, resolves and validates every candidate address against the same public-address policy before connecting, and refuses redirects or address changes that bypass that policy. This future reconnect gate must be implemented before imported hints can be used for requests; import never invokes the existing connection flow automatically.

## Proposed v1 envelope

```json
{
  "format": "beans-setup",
  "version": 1,
  "created_at": "RFC3339 UTC",
  "source": { "product": "Beans", "minimum_reader": 1 },
  "bots": [],
  "routines": [],
  "workflows": [],
  "provider_hints": [],
  "plugin_refs": []
}
```

This is a contract proposal, not an existing Beans wire format. Closed top-level envelope; bounded optional extension map is not allowed in v1. Readers reject unsupported `format`/`version`/`minimum_reader`, duplicate IDs/names where IDs are required, unknown enum values, malformed references, oversized/deep JSON and invalid paths before any write. Unknown optional fields within future explicitly extensible records should be retained on read/write; v1 records without extension maps reject unknown fields to prevent silent semantic loss. Limits: 1 MiB total UTF-8, 100 bots, 500 routines, 500 workflows, 100 provider hints, 100 plugin refs; 64 KiB per record, strings <=16 KiB, JSON depth <=32. No filesystem path is accepted as an import destination or executable field.

Export generates fresh random recipe-local record IDs or omits IDs; importer always allocates fresh account IDs and maintains an explicit old-to-new map for internal bot references. Name collisions never overwrite: preview offers create-with-distinct-name or skip; automatic suffixing is deterministic but user-visible before commit. There is no merge/replace in v1. Broken bot references reject the complete bundle, not silently drop linked routines/workflows. Active runner, source account/device IDs, pending jobs and deletion/tombstone state are forbidden fields.

## Export and import transaction contract

Exporter creates an explicit inventory and redaction/omission preview before writing. No credentials or credential-shaped fields may serialize; fail closed if a secret-bearing field appears. Export writes one bounded file to a user-selected destination using same-directory private temp, flush/sync, then atomic rename; cancellation or failure leaves prior destination unchanged. Plaintext recipe is not encrypted; users can protect it using existing OS disk/file encryption. Encryption belongs to a separately specified feature, not v1.

Import requires an existing, already paired and authenticated destination account to be selected before any staging or write. A Device that created or restored the account satisfies the same authenticated-membership precondition. Without that account context, import is refused with no write and no account creation, pairing or registration. The importer binds the preview to that account and rechecks it at commit; switching, forgetting or losing membership invalidates the preview and aborts the transaction.

Importer parses and validates entirely before state mutation, then shows resolved create/skip choices, endpoint/plugin hints, missing dependencies and explicit exclusions. It stages candidate bots, disabled routines and non-executable workflow definitions in one local transaction (or equivalent all-or-nothing durable commit); any error/cancellation leaves roster unchanged. It does not perform network access during validation/preview/commit. After commit, optional installs, provider reconnects, path remapping and routine activation are separate explicit actions. Unsupported versions and unsafe paths are no-write outcomes.

## Compatibility and dependencies

Recipe version is independent of account format and relay protocol. No protocol change: current roster remains protocol 5 / `beans-v2`. Recipe IDs are not authority. Target-account IDs always win. Across clients, importer must use shared Rust validation and transaction semantics; clients only render preview/collect choices. Feature must not conflate with #150 content export or #154 Runner backup. Profile, connection, memory and grant semantics remain subject to #107; until settled, v1 carries no profile-scoped grant or connection instance references.

## Reproducible fixture walkthrough

Fixture: two bots (one with `look`, custom provider/model hint, plugin ref), one routine referencing bot A, and one workflow referencing bot B. In an existing authenticated target account with an empty roster, expect fresh IDs and remapped references; the routine has `is_enabled: false`, no pause reason/check/run outcome, and no next scheduled run. The workflow is a `WorkflowRecord` definition only, with no job or routine trigger. Reopening the account leaves both non-running. The endpoint remains unconfigured and the plugin requires explicit trusted-marketplace installation. No secrets, grants, memory, chat data, paths, source identity, or machine identity transfer.

Missing/unauthenticated target, account switch after preview, malformed reference, unsupported version, over-limit input, path traversal or injected secret field are no-write outcomes; a collision follows the explicit create/skip choice. Exercise `http`, URL credentials, invalid ports, loopback/private/link-local IPv4 and IPv6 literals, and ambiguous encodings as local endpoint rejections; assert zero resolver calls and zero sockets even for a valid DNS hostname during import. At separate reconnect, a hostname resolving to a private address is rejected before HTTP. Commit failure/cancellation leaves the target unchanged. These are design-selected outcomes; no implementation or runtime scenario was executed.

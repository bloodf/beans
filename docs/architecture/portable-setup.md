# Portable setup contract — discovery outcome for #220

This is a configuration recipe, not an account backup, content export, live-sync mechanism, Runner clone, or second identity. Existing-account setup uses Pair first: protocol-5/`beans-v2` pairing preserves existing account sync semantics. Importing the recipe into another account creates configuration only; it never carries identity, permissions, credentials, content, execution state, or machine-local authority.

## Ownership and inventory

| Data | Owner today | Recipe policy | Import effect |
|---|---|---|---|
| Account identity, master secret, device/machine keys, account DEK, relay bearer | Identity device / each paired Device | Exclude | Target account remains selected; never pair or register implicitly |
| Bots: name, description/instructions, provider/model/thinking identifiers, `look`, legacy symbol/accent | Encrypted account roster; appearance contract in `avatars.md` | Include only these profile fields, bounded and schema-versioned | Create new bot IDs; never copy source IDs or overwrite existing profiles silently |
| Routines and saved workflow definitions | Encrypted account roster (`bots.md`) | Include definitions only; omit run state, checks, logs, timestamps, outcomes | Create paused routines; workflows remain inert definitions; require explicit enablement separately |
| Capabilities / grants, Auto-review, account Pause, policy receipts | Encrypted roster/policy | Exclude | New bots receive least capability defaults; no imported grant or approval |
| Provider IDs/endpoints and model IDs | Bot profile plus account credentials (`providers.md`) | Include nonsecret connection hints only; never keys, OAuth, refresh tokens | Treat as unconfigured; user explicitly reconnects; validate endpoint on selected Runner |
| Plugins, manifests, OAuth/API credentials, variables, installed state, catalog/tool cache | Runner-local `plugins/` | Include only stable marketplace plugin IDs and optional version/pin hints; no custom executable manifests or variables | Show preview; explicit install from trusted current marketplace, then separate setup/sign-in and bot grant |
| Skills | Runner-local managed library (`skills.md`) | Exclude bytes and source paths; optional opaque reference is informational only | No copy or activation |
| Memory service connections/preferences | Account-encrypted `memory_config`; paths and bindings Runner-local | Exclude; profile contract #107 governs future profiles | No service binding or consent copied |
| Bot `MEMORY.md`, topic notes, diary, chats, attachments | Runner-local memory or account content | Exclude; content export is #150 | No content import |
| Local paths, filesystem roots, embedding assets, MCP server config, machine metadata | Runner-local | Exclude | User remaps and configures locally |
| VM disks/controllers, jobs, approvals, pending requests, routine checks/runs, push tokens, relay queues/caches | Device/Runner/relay ephemeral or local | Exclude | Never cloned or resumed |

Definitions are bounded inert data. Import must not run hooks, scripts, routine checks, plugin servers, workflows, or model calls. Marketplace references do not imply trust or installation. Provider endpoints are untrusted hints requiring validation and explicit confirmation; secrets are always re-entered or connected through existing auth flows.

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

Importer parses and validates entirely before state mutation, then shows resolved create/skip choices, endpoint/plugin hints, missing dependencies and explicit exclusions. It stages candidate bots and paused routines in one local transaction (or equivalent all-or-nothing durable commit); any error/cancellation leaves roster unchanged. It does not perform network access during validation/preview/commit. After commit, optional installs, provider reconnects, path remapping and routine activation are separate explicit actions. Unsupported versions and unsafe paths are no-write outcomes.

## Compatibility and dependencies

Recipe version is independent of account format and relay protocol. No protocol change: current roster remains protocol 5 / `beans-v2`. Recipe IDs are not authority. Target-account IDs always win. Across clients, importer must use shared Rust validation and transaction semantics; clients only render preview/collect choices. Feature must not conflate with #150 content export or #154 Runner backup. Profile, connection, memory and grant semantics remain subject to #107; until settled, v1 carries no profile-scoped grant or connection instance references.

## Reproducible fixture walkthrough

Fixture: two bots (one with `look`, custom provider/model hint, plugin ref), one routine referencing bot A, and one workflow referencing bot B. Expected inventory: those inert definitions included; IDs remapped; routine disabled; workflow inert; endpoint shown as unconfigured; plugin requires explicit trusted-marketplace install; no secrets, grants, memory, chat data, paths, source identity, or machine identity. Preview malformed ref, unsupported version, over-limit input, path traversal, injected secret field, or collision: reject/skip as specified before writes. Commit into empty target yields fresh bot IDs, internal references remapped, routine paused, workflow inert. Commit failure/cancel yields unchanged target. These are design-selected outcomes; no implementation or runtime scenario was executed.

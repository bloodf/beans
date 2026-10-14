# Computer ownership

**[#34](https://github.com/bloodf/beans/issues/34) discovery; not implemented.** Proposed behavior below is not shipped API or verified virtualization. No prerequisite issue. Cross-P8 reconciliation gates closure, not implementation. #35 owns [display transport](relay.md), codecs/network/controller framing; this subject owns authority/config/status/admission. #339/#341/#343 remain remote-owned.

## Current evidence and boundary

Source at `755ecf123564abd8ef8c9657e04e4d9b3be7d7a2`: [model.rs](../../crates/cli/src/model.rs) (`Capabilities/Bot/RosterBlob/MachineBlob/Request/Response`), [sync.rs](../../crates/cli/src/sync.rs) (`merge_rosters`), [requests.rs](../../crates/cli/src/requests.rs) (`ask_within`, signatures), [relay client](../../crates/cli/src/relay.rs), [relay routes](../../crates/relay/src/routes.rs). Current ownership/transport: [Account model](account-model.md), [Identity](identity.md), [Bots](bots.md), [Phone core](phone-app.md).

Current: one Runner per bot; phones/tablets never run bots. Strict `Capabilities` accepts `shell/write/plugins` only, defaults true/true/unrestricted, rejects unknown fields. Typed Bot round trip discards unknown fields. Roster merge replaces whole `capabilities`, not individual permissions; versioned policy projects separately. `MachineBlob` has `device/turns`, no lifecycle. Signed paired requests seal to Runner, reject unknown/offline targets, wait 20 s. Protocol/floors **5**, **Beans-Protocol/Beans-Format**, `beans-v2` supersede issue baseline's historical protocol 4/`Lorca-Protocol`. Kinds are closed, including current memory config extension; proposed design adds none.

Proposed invariant: one computer per bot on its assigned **self-hosted Linux Runner**, a Linux VM only. QEMU/KVM through libvirt is candidate, unverified; feasibility/probe evidence must confirm it. No containers, host execution fallback or guest execution when KVM admission fails. Runner remains available control plane and agent loop while guest is stopped. Guest receives scoped computer access only: no account master/DEK/machine keys, full provider credential blob, relay storage or host filesystem mounts. No billing/usage metering; owned-server and retained-disk costs continue while stopped. Stop retains disks, not RAM/application state. Native implementation only; no dependency/code imported here; GPL-3.0 constraints prohibit copying restrictive bops FSL-1.1-ALv2 or openbot PolyForm Noncommercial code.

## Proposed ownership and storage

Encrypted roster holds per-bot desired `computer` config and `capabilities.computer`, never plaintext relay columns. Ownership binds account, stable `bot.id`, `runner_id`, `generation`; rename never moves disks. Absent config disables provisioning. Config replacement validates expected revision separately from lifecycle commands. Proposed shape uses illustrative IDs:

```json
{"computer":{"version":1,"enabled":true,"generation":"generation-id","revision":{"counter":1,"device_id":"device-id"},"resources":{"vcpus":2,"memory_mib":2048,"disk_gib":20}},"capabilities":{"shell":false,"write":false,"plugins":[],"computer":true}}
```

Resources: positive integers within Runner-advertised maxima/available capacity; feasibility reconciliation supplies exact limits/probe thresholds. Reject client host paths, VM XML, mounts, commands, URLs and provider fields. Private Runner inventory maps ownership to VM/domain/disks, image provenance, effective limits, generation, operation journal and cleanup tombstones; never sync it or expose it in status/public reports. Persist provisioning journal/config receipts before acknowledgment; recover partial provisioning without claiming `running`. Status may carry approved opaque provenance references, never secret/path/provenance internals.

Reject Runner reassignment while retained resources exist: changing assignment is not disk migration. Stop/export/import/provenance requires separate lifecycle reconciliation; no automatic transfer/host fallback. Disable revokes access/stops guest but retains disks; deletion requires separate confirmation. Re-enable retains disk/generation unless deleted, never auto-starts.

New cores preserve unknown config/status extension fields in stored JSON and fieldwise merge. Config `computer` is independent of profile, avatar and other permissions; an unrelated edit preserves remote config. Resource/config replacement uses revision conflict detection, not arbitrary last-writer overwrite. `capabilities.computer` has its own policy version: concurrent shell/write/plugin edits must not restore it. Computer enable/access edits and deletion enter encrypted append-only `policy` actions; roster carries their reconciled versions/tombstones. A later explicit grant may supersede revocation; unrelated/offline edits never do. Unknown capability permissions/versions confer no authority; retain opaque data but refuse unsupported execution, never change unknown to allowed. These are required new merge/admission rules, not claims about current merge.

## Proposed status and asynchronous commands

Extend existing encrypted Runner `machine` with `computer_protocol: 1` and `computers`; retain slot/DEK/kind AAD, add no relay kind. Runner alone reports actual state; roster is intent. No disks/framebuffer/input/session secrets in roster/machine. Proposed entry:

```json
{"computer_protocol":1,"computers":[{"bot_id":"bot-id","generation":"generation-id","config_revision":{"counter":1,"device_id":"device-id"},"status_seq":7,"state":"starting","observed_at":0,"operation":{"id":"operation-id","kind":"start","state":"pending"},"reason":null}]}
```

Lifecycle: `disabled/unavailable/stopped/starting/running/stopping/deleting/deleted/failed`. Operations: `pending/running/succeeded/failed/cancelled`. Bounded reason codes exclude raw libvirt errors/host identifiers; `unavailable` includes unsupported/non-Linux/KVM/probe failure. `status_seq` increases per generation; timestamps are display evidence, not authority. Snapshot/bootstrap includes last status plus `stale`/reachability; `computer.changed` emits same sanitized entry. Disconnected/cached `running` never proves liveness or authorizes input. New generation invalidates old status/sessions; duplicate/out-of-order updates cannot regress sequence. Bootstrap repairs missed events.

Mutations return durable **admission receipts within 20 s**, not boot/shutdown completion. Client chooses operation id; exact id/body/binding replay returns same receipt, changed intent rejects. Serialize per computer; stop/deletion supersedes start. Persist outcomes across restart; inspect actual domain, never blindly reboot. Timeout means **outcome unknown**, not rollback/new-id retry; query original id after reconnect. Delayed replies cannot recreate authority. Proposed envelope retains signed `beans-request-v1` and request/response kinds:

```json
{"id":"request-id","verb":"computer.start","requested_by":"device-signing-key","created_at":0,"body":{"payload":{"version":1,"bot_id":"bot-id","generation":"generation-id","expected_revision":{"counter":1,"device_id":"device-id"},"operation_id":"operation-id","expires_at":20},"signature":"signature"}}
```

Proposed response body: `{ version: 1, operation_id, accepted: true, state: "pending", generation, config_revision }`; `accepted` means persisted admission, not completion. Query returns terminal outcome or explicit unknown id. Deadline bounds admission: reject expired queued requests, and requests implausibly future-dated under the reconciled clock tolerance. Accepted operations can outlive it; cancellation remains explicit. Exact time/clock policy needs threat-model reconciliation. Signature binds whole payload; Runner also verifies current membership, account incarnation, assignment, generation, revision, protocol, Pause and authority at admission and before each effect/async completion.

## Proposed client methods and events

All AppKit, Windows/Linux and phone/tablet stores use one core schema. Desktop websocket retains existing envelope; phone UniFFI `Core.request(method, params)` and event listener expose same methods without a new native VM runtime. Every method requires version 1, bot id and expected Runner/generation binding where established; routing resolves roster ownership and rejects mismatch rather than forwards to arbitrary host. Config before initial provisioning uses revision/assignment binding without a generation; Runner creates generation on first admitted provision.

| Local method / sealed verb | Params beyond binding | Result / semantics |
| --- | --- | --- |
| `computer.get` | none | Desired config, sanitized last-known status, stale/reachability, supported actions; authoritative refresh if reachable |
| `computer.configure` | `expected_revision`, complete enabled/resources draft, `operation_id` | Revision/admission receipt; validate/provision asynchronously |
| `computer.start` | `expected_revision`, `operation_id`, `expires_at` | Admit start; no boot wait |
| `computer.stop` | same | Admit stop; retain disk, discard RAM |
| `computer.operations.get` | `operation_id` | Durable operation/status or unknown id |
| `computer.operations.cancel` | `operation_id` | Cancel pending work; running irreversible effect may finish; report actual result |
| `computer.delete` | revision, operation id, one-use confirmation bound to exact disk generation | Admit stop and disk/inventory deletion; cleanup status remains visible |
| `computer.view.open` | `mode: view/control`, revision, short-lived requester-bound intent | #35 session grant; no grant in machine blob |
| `computer.view.close` | `session_id` | Revoke own grant, release controller and queued input |

The destructive confirmation is minted through `computer.delete.preview` (same binding); returns sanitized retained-data summary plus one-use token, never private disk paths. Preview/confirmation is trusted user flow, not bot tool. Token invalidates on account/assignment/generation/revision change or attempted consumption. Errors preserve existing `{ error: { message } }` envelope with bounded codes: `computer_unsupported`, `computer_offline`, `computer_paused`, `computer_revoked`, `computer_conflict`, `computer_expired`, `computer_operation_unknown`, `computer_confirmation_required`, `computer_cleanup_pending`. Unsupported method follows current `unknown method <name>`; UI disables action rather than retries a different method. No hidden host-shell fallback.

`computer.changed` and `computer.operation.changed` events/snapshot entries expose binding, status seq and sanitized outcomes; `computer.view.closed` reports revocation/disconnect. Clients refresh after timeout/foreground/reconnect and clear all cached computer/session state on identity/relay-source change. Stop/delete remain available as safety actions while Pause/access is revoked; view/control/start/provision are blocked. Physical input uses #35's authenticated transport, **not** per-keystroke websocket RPC/sealed requests. Shared grants bind account, bot, Runner, requesting paired Device, generation, policy revision, mode and expiry; #35 owns controller arbitration, transport/key framing and disconnect behavior. No cached grant survives epoch/policy change or is reused for another computer.

## Proposed capability, Pause and revocation

`computer` defaults **false**, independent of shell/write/plugins: those existing permissions grant no guest access and disabling computer does not silently disable host tools. Agent computer calls require computer=true, enabled config, correct Runner/generation and healthy admission. Trusted paired-user lifecycle/config/view controls require explicit computer enablement and current policy, not model possession of a grant. Guest accepts only scoped, expiring Runner-issued commands/session capabilities; agent loop/provider use stays on Runner.

Upon local acceptance of Pause, computer revocation, disable, bot removal, requester unpair or account teardown: fence admission first, invalidate grants/controller leases, discard queued input, cancel pending provision/start/tool/view work, then stop running guest (disk retained except deletion). Check fence before irreversible async effects and before publishing completion. A boot already issued may finish physically, but must not become usable or receive new input; stop it and report actual cancellation/cleanup. Resume/regrant never replays canceled input or automatically restarts guest. Paired membership must be rechecked during sessions, not only initial request.

Propagation is bounded by **observation, not a promised wall-clock remote deadline**. Local fence takes effect before acknowledgment; remote Runner fences immediately when reconciled policy/membership arrives. Offline Runner cannot learn remote revocation. Proposed computer execution is fail-closed while relay/policy freshness is uncertain: close sessions, reject effects, stop guest when control-plane sync is lost; keep Runner alive for recovery. On reconnect, pull membership/roster/policy and apply tombstones before processing queued commands or reopening sessions. Exact freshness/watchdog thresholds require feasibility/threat-model reconciliation. Already-issued external effects cannot be undone; stopped disks retain writes. Do not claim offline cleanup or instantaneous network revocation.

## Proposed compatibility, migration and offline matrix

Nested unknown-field rejection plus unknown Bot-field loss prohibits additive protocol-5 enablement. Reserve **mandatory computer floor P, P > 5**, not a numeric allocation: staged remote protocol-6 ownership exists independently and must not be repurposed. Supervisor reconciles number/health fields with #339/#341/#343 and other P8 outputs before activation. Proposed health advertises supported `computer_protocol: 1`, enforced `min_protocol/min_roster_protocol: P`; all account mutation, read, pairing and socket routes require admitted P, not just roster PUT. Relay still cannot decrypt values and adds no blob kind. New cores reject absent/wrong floor or computer capability evidence before pulling/publishing extended state. Headers remain `Beans-Protocol` and `Beans-Format`; exact format/home marker transition is reconciled with storage owner, never silently assigned by this doc.

| Device/core and relay | Behavior |
| --- | --- |
| Current core + current protocol-5 relay | Current behavior unchanged; no computer APIs/state |
| Current core + computer-enabled P relay | Existing health floor check fails; account routes return `426`; no computer roster write/input |
| New core + protocol-5 relay or missing computer evidence | Ordinary non-computer mode only before account activation; no extended roster/policy or computer commands; activated account fails closed |
| New core + admitted P relay + old UI | Core refuses unknown/unsupported computer intents; old UI ignores optional status, has no controls; cannot weaken permission by whole-field save |
| New core + P relay + unsupported/absent Runner machine capability | Show unavailable/unknown last state; no start/view/input or host fallback |
| New core + P relay + supported Linux Runner + current UI | Version-1 config/status and signed async operations, subject to live policy/probe/session gates |
| Unknown computer/config/session version | Preserve opaque stored extensions; display unavailable; reject effects/input, never reinterpret as version 1 |
| Offline Device or Runner | Display stale status; new commands fail visibly, not silently queued; accepted operations remain journaled/queryable |

Activation is explicit after every paired writer/core and relay meets P and storage admission has proved downgrade protection. Existing bots get absent config/default false; no disk creation or permission expansion. Old offline outboxes are not replayed into extended roster: upgraded core reconciles frozen baseline under new merge/policy rules or stops with conflict retaining original bytes. New local admission marker prevents an old binary opening an activated home; exact marker must be agreed with storage owner and independently proved. Merely raising relay floor does not protect old local cached state. No mixed-account auto migration/reset, key replacement, or deletion to “fix” incompatibility. Rollback before activation keeps existing data; after activation unsupported binaries/relay reject until a separately validated downgrade/export, never strip fields. Fixture proves current hazards, not this proposed migration.

No offline command queue is offered: unknown/offline/missing-key/no-relay targets return visible reason before enqueue. A connection race after upload can leave sealed request queued, so admission expiry and durable idempotency prevent surprise late starts; timeout shows unknown outcome until query. Offline status is not authority. Already admitted work is canceled/fenced on loss of required freshness and reconciled from journal on restart; no claim that remote cancellation succeeded until Runner evidence arrives.

## Proposed deletion and closure evidence

Bot deletion's durable policy tombstone dominates config/status. Runner fences work, closes sessions, stops guest, destroys bot-owned disks/snapshots and private inventory, then records cleanup outcome; clients retain cleanup-pending evidence rather than declare success while Runner is offline. Late callbacks/status cannot resurrect removed ownership. Chat deletion alone never deletes another bot's computer. Separate computer deletion retains bot, removes its computer resources/config enablement and advances generation/tombstone; retained disk cannot be silently rebound to a recreated bot/id.

Account deletion still requires successful relay deletion before current local forget. Proposed teardown additionally fences local computer work and purges reachable local guest resources/journals/session secrets. Other/offline Runners may retain disks until they observe revoked account/`410`; publish no promise of remote disk erasure from relay deletion. Local account forget/unpair fences then deletes that Runner's account-owned computer resources with durable retry evidence; interrupted cleanup blocks replacement-account reuse. Never transfer resources/secrets across account incarnation. Host filesystem snapshots/backups and SSD secure erasure are operator responsibilities, not guaranteed by opaque relay deletion. Stop is non-destructive, delete requires explicit retained-data confirmation, and failed cleanup remains visible/retryable without restoring authority.

The recorded source-extracted old-client fixture below proves default preservation, nested unknown-field rejection, Bot extension loss and whole-field capability merge; it adds no permanent source tests or Cargo build. Evidence excludes private host paths. `bun run check:docs` validates size/links only, not VM feasibility, client implementations or future floor safety.

Before closure, supervisor must record reconciliation against probe thresholds, image provenance, threat model, lifecycle, controller, ownership and #35 display outputs; resolve P/storage admission, freshness/clock/resource limits and shared session binding. Downstream API/client issues must reference this section and receive changed-input notes for whole-field permission merge, mandatory floor, async receipts/20 s, cleanup pending and transport authority. Those updates and merged/recorded decision links remain acceptance evidence, not fabricated success or prerequisites. No GH lease, implementation/build, live VM, installed-client, display or deletion verification is claimed by this discovery contract.

## Recorded compatibility fixture

At the source revision above, rustc `1.99.0 (b940084d7 2026-09-28)` (Homebrew) compiled and ran this isolated fixture on darwin 25.6.0 arm64; Bun `1.4.2` is available. Existing serde artifacts were linked, without Cargo. `Attachment`/`BotLook` are JSON substitutes for unrelated fields; `Roster` is minimal. Capabilities declaration/default, Bot declaration and bot merge macro/invocation are source-extracted, not a reimplementation. This exercises deserialization and bot field merge, not policy projection, whole-core startup or installed clients.

SHA-256 source bindings:

- `crates/cli/src/model.rs`: `a3b15eea514e058df8ad3773a86fa2cc161935cd1832d73dbd7ccb9aea7bbbce`
- `crates/cli/src/sync.rs`: `20c213aa779bc431f14dad7f6db2c05aab07df599c193677d038cb1685411b4d`
- Fixture: `f686d817c5c3a039b35d5ada8f050d9f13548d3a7f7588f138e4d6200ec7ecb2`

Write the following unchanged to `$SCRATCH/fixture.rs`; set `DEPS` to the existing matching Rust artifact directory. No build of Beans is required.

```sh
rustc --edition=2021 "$SCRATCH/fixture.rs" -L dependency="$DEPS" --extern serde="$DEPS/libserde-492747a8269cbb1d.rlib" --extern serde_json="$DEPS/libserde_json-8466cd8d0e3ab640.rlib" -o "$SCRATCH/fixture" && "$SCRATCH/fixture"
```

Raw output, exit 0:

```text
PASS missing capabilities keeps shell/write/plugin defaults
PASS nested computer capability rejects: unknown field `computer`, expected one of `shell`, `write`, `plugins`
PASS unknown bot-level computer field is discarded on typed round trip (unsafe rollout)
PASS unrelated name edit preserves remote capability restriction
PASS concurrent capability edit replaces whole capability field before policy projection; not per-key merge
```

Generate the source-extracted fixture from the pinned checkout with Python 3, then move `fixture.rs` into `$SCRATCH` before the compile command above. The generator preserves the recorded fixture bytes, including source comments.

```python
from pathlib import Path
model = Path("crates/cli/src/model.rs").read_text()
sync = Path("crates/cli/src/sync.rs").read_text()
cap = model[model.index('#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\n#[serde(default, deny_unknown_fields)]'):model.index('\nimpl Capabilities {')]
bot = model[model.index('#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]\npub struct Bot {'):model.index('\n/// One Auto-review')]
merge = sync[sync.index('    macro_rules! merge_entities {', sync.index('fn merge_rosters(')):sync.index('    merge_entities!(chats,', sync.index('fn merge_rosters('))]
prefix = 'use serde::{Serialize,Deserialize};\nuse serde_json::{Value,json};\ntype Attachment = Value;\nmod appearance {pub type BotLook = serde_json::Value;}\n'
harness = '\n#[derive(Clone)] struct Roster {bots: Vec<Bot>}\nfn merge(mut remote:Roster,base:&Roster,queued:&Roster)->Roster {\n'
tests = 'fn main(){\n let raw=json!({"id":"bot-fixture","name":"base","description":"","symbol_name":"","accent":"","runner_id":"runner-fixture","provider":"fixture","created_at":0});\n let baseline:Bot=serde_json::from_value(raw.clone()).unwrap();\n assert_eq!(baseline.capabilities,Capabilities{shell:true,write:true,plugins:None});\n println!("PASS missing capabilities keeps shell/write/plugin defaults");\n let mut nested=raw.clone(); nested["capabilities"]=json!({"computer":true});\n let err=serde_json::from_value::<Bot>(nested).unwrap_err(); assert!(err.to_string().contains("unknown field `computer`"));\n println!("PASS nested computer capability rejects: {}",err);\n let mut top=raw; top["computer"]=json!({"version":1,"enabled":true});\n let old:Bot=serde_json::from_value(top).unwrap(); assert!(serde_json::to_value(old).unwrap().get("computer").is_none());\n println!("PASS unknown bot-level computer field is discarded on typed round trip (unsafe rollout)");\n let base=Roster{bots:vec![baseline.clone()]}; let mut remote=base.clone(); remote.bots[0].capabilities.shell=false;\n let mut local=base.clone(); local.bots[0].name="renamed".into();\n let merged=merge(remote.clone(),&base,&local);assert_eq!(merged.bots[0].name,"renamed");assert!(!merged.bots[0].capabilities.shell);\n println!("PASS unrelated name edit preserves remote capability restriction");\n local.bots[0].capabilities.write=false;let merged=merge(remote,&base,&local);assert!(merged.bots[0].capabilities.shell);assert!(!merged.bots[0].capabilities.write);\n println!("PASS concurrent capability edit replaces whole capability field before policy projection; not per-key merge");\n}\n'
fixture = prefix + cap + bot.replace('crate::appearance::BotLook', 'appearance::BotLook') + harness + merge + 'remote}\n' + tests
Path("fixture.rs").write_text(fixture)
```

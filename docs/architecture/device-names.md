# Device names

A Device's machine metadata holds its original name. The account's encrypted roster adds `device_names`, a map from Device id to account alias. Absent maps decode as empty. `App::device`, snapshots and roster events use the alias as `name`; known Device rows also expose the underlying `machine_name`. Unknown Devices keep their existing unknown presentation. Display sorting uses the effective name.

## Local methods

| Method | Params | Result |
| --- | --- | --- |
| `device.rename` | `name` | `name`; renames this Device's machine metadata |
| `device.set_name` | `id`, `name` | `null`; sets a known Device's account alias |

`device.set_name` trims whitespace and accepts at most 64 Unicode scalar values. Blank text or the current machine name removes the alias. Missing or non-string names answer `missing name`; the id uses the local API's required-string validation. An unknown Device or unavailable account rejects before persistence. Existing clients retain `device.rename`; it changes the underlying machine name even when an alias currently masks it.

## Persistence and synchronization

Alias edits acquire the roster edit lock before account lifecycle admission. The state snapshot, encrypted latest-roster outbox item, applied receipt and frozen ancestor commit in one SQLite transaction before memory changes or events. State or queue failure leaves the previous alias and pending intent unchanged. Restart recovers the same queued intent. Forget clears the alias table with the rest of the account state.

Roster CAS rebase applies only alias keys changed relative to the frozen baseline, preserving unrelated remote edits and resets. Before upload, the submitted roster remains durable. A known accepted upload advances the alias ancestor for its submitted keys. A later remote roster after a lost response proves acceptance only for matching keys; one matching alias does not acknowledge every submitted alias. Newer local resets survive recovery.

Unpair keeps its account and relay ownership checks. Local membership removal, live-turn purge and any alias-removal roster intent commit together. After remote revocation, storage failure keeps membership removed in memory, retains the alias intent for retry and returns an error; retry completes persistence before dropping the retained alias. Notifications do not write account state.

These fields do not negotiate a new relay protocol. A core predating `device_names` can discard this map when publishing its roster. Alias preservation therefore requires all roster-writing Devices to run an alias-aware core. The additive local method retains existing app compatibility; alias editors are separate client consumers.

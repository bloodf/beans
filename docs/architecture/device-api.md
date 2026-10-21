# Device API and account aliases

These methods use the [local API connection](local-api.md#connection-and-envelope).

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `device.set_name` | `id`, `name` (trimmed, at most 64 Unicode scalar values; blank resets to machine name) | `null`; unknown Devices reject |
| `device.unpair` | `id` (this Device's id forgets the identity) | `null` |
| `device.update`, `device.auto_update` | any | always the error `SELF_UPDATE_UNAVAILABLE` |
| `device.service_status` | see [Routine and background service API](routine-api.md) | service state |

## Alias persistence and sync

Device naming uses `device.set_name { id, name } -> null`. The encrypted roster's optional `device_names` map stores account aliases; absent entries use original machine names. Device rows in snapshots and `roster.changed` expose effective `name` and original `machine_name`. Alias edits and explicit resets participate in per-key fieldwise roster CAS rebase and persisted submitted-baseline recovery, not whole-map latest-wins replacement.

When a later roster from another Device supplies lost-response recovery evidence, only alias keys matching the submitted set/reset advance the alias baseline. A matching key does not establish acceptance of other submitted aliases. A directly acknowledged or recognized own roster retains its atomic acceptance boundary.

Device alias edits hold the account lifecycle boundary before the roster edit lock. The alias state, encrypted roster outbox item, frozen queued ancestor, and applied-blob marker commit in the existing fallible SQLite queue transaction. Memory changes, outbox notification, and roster events publish only after that transaction succeeds. Encryption or persistence failure returns an error without publishing the staged alias; no separate journal or startup-admission exception is used.

Unpair captures the local account incarnation and machine identity before relay awaits and rechecks them under the lifecycle boundary before local cleanup. After relay revocation, local membership removal is immediate and remains fail-closed even if persistence fails. Alias removal is staged with its encrypted roster reset, queued ancestor and applied marker in the existing queue transaction; persistence errors propagate without claiming to undo relay revocation. A failed transaction retains the prior alias and queued intent for a retry, but does not restore local membership. Lifecycle, roster and state guards release before notifications, roster events and turn callbacks. A stale completion cannot remove membership from a replacement account.

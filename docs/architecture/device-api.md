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

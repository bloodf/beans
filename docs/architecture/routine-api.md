# Routine and background service API

These methods use the [local API connection and trust boundary](local-api.md#connection-and-envelope). The CLI owns scheduling and persistence; [Bots, Routines, and Memory](bots.md#routines) describes execution and recovery.

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `routines.create` | `bot_id`, `name`, `schedule`, `prompt?`, `enabled?`, `timezone?`, `missed_run_policy?` | `routine` |
| `routines.update` | `id`, `name?`, `schedule?`, `prompt?`, `enabled?`, `timezone?`, `missed_run_policy?` | `routine` |
| `routines.delete`, `routines.run` | `id` | `null` |
| `routines.describe` | `schedule`, `timezone?`, `missed_run_policy?` | `schedule`, `text`, `timezone`, `missed_run_policy`, `next_run_at` |
| `device.service_status` | `id?` (Runner id; this Device when omitted) | `installed`, `running` (booleans); local inspection and remote requests retain Runner update admission |

## Routine fields

Routine timezones are IANA names; creation and description default to this Device's zone, with UTC when unavailable. `missed_run_policy` is `coalesce` (default) or `skip`. Updates keep omitted values. A routine output includes `timezone`, `missed_run_policy`, optional `last_scheduled_at` and `health`, plus `schedule_text`, nullable `next_run_at`, `is_running` and `state` (`running`, `blocked`, `paused`, `waiting_for_runner`, `failed`, `on`). `paused_reason` is optionally `away` or `authentication`. All routine timestamps are Unix seconds.

Optional `pending` is a Runner-owned scheduled intent with `job`, `phase` (`check`, `checking`, `model`, `running`), and optional `binding` (bot, schedule, timezone, missed-run policy, check, re-arm time and claimed cursor). Clients cannot edit it. Unstarted bound intents recover after restart; effect-started or missing-binding intents remain `blocked` without automatic replay. Inactive-to-enabled Resume retains and explicitly reauthorizes unstarted work, but discards uncertain effect-started intent. Redundant enable does neither. Missing-binding intent requires explicit pause then resume. Run Now refuses pending intent. See [routine recovery](bots.md#routines).

`health` contains `updated_at`, nullable `last_check_at`, `last_success_at`, `status` and `retry_at`, integer `connection_failures` and `authentication_failures`, and `model`. `model` contains nullable `status` and `retry_at` plus its own integer failure counters. Both statuses are `quiet`, `ready`, `failed`, `blocked` or null. Check success does not reset model failures.

## Background service routing

`service.status` is a signed sealed Runner verb, not a local API method; its body is empty and its result matches `device.service_status`. There is no local `service.status { runner_id }` endpoint. Service installation and ownership follow [CLI service and updates](service-updates.md).

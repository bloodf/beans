# Attention

The account's encrypted coordinator attention projection. Task and review ids are links, never
duplicate work records. Reporter observations merge by a stable underlying key.

## What it is

Attention is one list of what needs the user across every chat: a review waiting for a decision, a
blocker, a commitment a bot made, an important change. Coordinating bots keep briefs of decisions
and changes. Every Device shares the same encrypted projection through `attention` blobs.

## Projection

Each observation, brief, preferences record, and resolution is an encrypted SQLite row under
`attention(id, ciphertext)` and an account-encrypted relay blob with a latest-only slot. The view
projects active items (urgent first) and each coordinator's current brief. A brief that links
`item_ids` leaves once none of those items is active; the user's `attention.dismiss_brief` marks
it dismissed under a new revision so it stays off the list until the coordinator publishes different
words. Identical publications after a dismiss stay dismissed.

## Reporting and resolution

`attention.report` saves a reporter's observation under a stable topic key. The actor is the
reporting bot, or `None` for the user. Ordinary reports wake the coordinator; urgent ones also post
a directly alerting source-chat message. Quiet reports publish no alert. `task_id` and `review_id`
are opaque links: Beans stores them without requiring durable-task or review-queue modules that
have not landed yet.

The coordinator, the bot that alone reported the item, or the user resolves with
`attention.resolve`. An optional `expected_revision` refuses a stale UI action. Resolved keys are
terminal. The `attention` tool is one of Beans's own tools on a Runner turn; every turn's prompt
lists the items that bot may resolve.

## Preferences and notifications

`attention.preferences` sets `summaries`, `urgent_direct`, and an optional
`default_coordinator_bot_id`. Structured text messages carry `notification: summary | urgent |
quiet`. Phone pushes honor that tag and the synced preferences. A turn that published a brief or
routed a report marks its other completed text as quiet so the user hears the news once.

## Local API

| Method | Params | Result |
| --- | --- | --- |
| `attention.list` | `include_resolved?` | `AttentionView` |
| `attention.report` | report fields | `Item` |
| `attention.resolve` | `id`, `expected_revision?` | `Item` |
| `attention.brief` | `coordinator_bot_id`, `decisions`, `changes`, `next_action`, `item_ids?`, `quiet?` | `Brief` |
| `attention.dismiss_brief` | `coordinator_bot_id`, `expected_revision?` | `{}` |
| `attention.preferences` | `summaries?`, `urgent_direct?`, `default_coordinator_bot_id?` | `Preferences` |

Bootstrap includes `attention { items, briefs, preferences }`. `attention.changed` replaces that
projection. Devices take attention blobs in live pulls and during first sync. A Device upgrading
from a build that skipped this kind replays attention slots once and records a persisted
`attention_upgrade` marker when the replay finishes (or after first sync when `last_seq` was 0).
A missing account key or a relay error leaves the marker unset so the next start retries, but
does not abort the rest of sync. A blob that fails to decrypt under a present key is logged by
id, skipped, and the replay may still complete and set the marker. On a live pull the same skip
applies and `last_seq` moves past it, so there is no later upgrade replay for that blob once the
marker is set. `receive` loads only the incoming id, upserts ciphertext, and commits the relay
receipt in the same transaction as the row. Undecryptable neighbors are skipped. Rows stay
ciphertext at rest.

## In the apps

The phone opens Attention from the chat list beside Settings while something waits
(`mobile/app/attention.tsx`). A tap opens the item's chat, or when the item waits on an open review
that is not a draft card, the chat with that review's screen on top. A long press offers other
source chats, Mark as Resolved, and on a brief Dismiss.

Desktop Attention and Running tasks flyouts are tracked separately (D9 / #395); they call the same
CLI methods above.

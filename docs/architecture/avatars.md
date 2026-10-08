# Bot avatars

## Profile and photo

A bot's generated portrait is a deterministic Blobatar seeded by its stable `bot.id`. Editing generated settings never changes identity. The core carries an optional `Bot.look` alongside the independent `Bot.avatar` attachment. Legacy `symbol_name` and `accent` remain in profiles for wire compatibility and do not drive generated portraits.

An uploaded photo has priority. Its bytes travel as an account-encrypted `file` blob, without a chat group, and each Device fetches them by attachment id. Saving or resetting `look` keeps the photo; uploading or removing a photo keeps `look`. The generated rendering contract uses the saved look when photo bytes are unavailable, and the seeded portrait when no look is saved. Rust owns persistence and the API; this contract does not establish client renderer/editor integration or platform visual acceptance.

The Mac, phone, and Windows/Linux apps generate seeded portraits locally, offline. Clicking the avatar in the macOS inspector opens the Look sheet. The macOS Profile card keeps Name trailing-aligned and opens Description in its own editing sheet. On the phone, tapping the avatar in Details opens Look inside the same form-sheet stack (`app/chat-info`); Name is trailing-aligned, Description opens its editor, and Details has native Provider, Model, and Thinking menus under Runner.

## Saved look, version 1

`crates/cli/src/appearance.rs` defines the shared persistence shape:

```json
{
  "version": 1,
  "base": {
    "shape": "cloud",
    "background": "none",
    "tone": "pastel",
    "palette": { "head": "#ABCDEF" },
    "motion": true
  },
  "states": {
    "working": { "shape": "boxy", "expression": "thinking" },
    "error": { "expression": "sad" }
  }
}
```

`version` and `base` are required. Every appearance field is optional; `states` is optional and uses `idle`, `thinking`, `responding`, `working`, `waiting`, `retry`, or `error` keys. Each state inherits omitted fields from base, with palette inheritance per channel. Sparse values remain sparse in storage so inheritance survives round trips. Runtime activity, animation frames, and timestamps are not look fields.

| Field | Values / rendering default |
| --- | --- |
| `shape` | `round`, `organic`, `boxy`, `capsule`, `nub`, `cloud`, `droplet`, `hexagon`, `sun`, `triangle`; omitted uses the bot-id seed |
| `expression` | `idle`, `happy`, `sad`, `mad`, `surprised`, `wink`, `sleepy`, `smug`, `unsure`, `scared`, `love`, `shy`, `sick`, `thinking`; omitted base expression is idle |
| `background` | `none`, `square`, `circle`, `squircle`; omitted base background is transparent/none |
| `hue` | Finite number in `[0,360)`; omitted uses the bot-id seed |
| `tone` | `pastel`, `pale`, `mid`, `deep`, `bright`, `ink`; shared renderer representatives `.10`, `.28`, `.49`, `.71`, `.865`, `.965`; omitted uses the bot-id seed |
| `palette` | Optional `head`, `eye`, `bg` channels, each canonical uppercase `#RRGGBB` |
| `motion` | Boolean; omitted base motion is true; OS Reduce Motion takes precedence in the rendering contract |

The local API authors only these fields and supported version/enums. It rejects null subfields, non-finite/out-of-range hue, noncanonical colors, arbitrary traits, CSS/SVG, and function values. Unknown keys read from newer rosters at look, appearance, or palette level are retained through unrelated edits, SQLite reload, and re-encryption; this preservation does not make them authorable through the version-1 API.

## API and durable sync

`bots.create` accepts `look` or starts without customization. `bots.update` distinguishes:

- omitted `look`: keep saved generated settings;
- `look: null`: reset generated settings only;
- look object: replace the complete validated draft, including base and states.

Validation precedes avatar file storage, outbox writes, and profile mutation. `App::update_bot` validates a changed look on a candidate profile before committing it. Image storage and deletion continue to use the existing independent attachment path.

The bot profile lives in local SQLite JSON and inside the account-encrypted roster. Fieldwise roster reconciliation treats `look` as one replace/reset field: an unrelated offline rename retains the remote look, while a changed local look replaces that field without erasing a remote rename, photo, or description. Concurrent changes to the same look resolve through the queued local intent and conditional roster slot writes, not subfield patching. Frozen baseline, encrypted outbox, submitted snapshots, and CAS recovery retain intent across retries and restart. See [Protocols](protocols.md#cli--relay).

## macOS renderer, activity, and Look sheet

`Design/AvatarView.swift` shows a bot's generated portrait through `AvatarRenderLayer` (`Design/AvatarRenderLayer.swift`): `CAShapeLayer`s for background, body, droplet taper, petals, and eyes, built from numeric frames. `Design/AvatarGeometry.swift` evaluates `packages/beans-blobatar/dist/blobatar.jsc.js` (copied into Beans and Beans Dev resources by `scripts/app.ts`) once in a lazy JavaScriptCore context, checks that the bundle installs the geometry globals, caches endpoint geometry by bot ID, canonical look, and state, and converts `avatarFrame` output to `CGPath`s and matrices. No SVG is parsed or decoded while animating. A state change morphs from the geometry on screen.

One `AvatarClock` drives visible portraits and stops when none move. Offscreen, hidden, or occluded portraits, Reduce Motion, and a look with motion off draw the static frame (`amp` 0), without morph transitions. Presence pulsing and the editor's preview-cycle timer also stop when hidden or motion is reduced. `AvatarClusterView` gives each generated slot its own layer, with rings cut out under front avatars and the working dot. A fetched profile image is clipped to a circle and wins; a generated portrait keeps its selected background shape.

The opt-in desktop native window evaluates the same `packages/beans-blobatar/jsc.ts` bundle in an isolated Go JavaScript runtime. `desktop/native_avatar.go` converts shared `avatarFrame` numeric contours and matrices into MyGo paths; it does not generate a second silhouette or parse SVG. Cached static fallback frames use amplitude zero. Uploaded photos fetched through the admitted CLI `files.path` request override generated geometry, keyed by the exact bot and attachment ID. Stale account/profile completions cannot install photos. The working Solid renderer/editor remains the default client while native platform parity is verified.

The native transcript opens `nativeLookEditor` through `native_look_integration.go`. Its callback captures account epoch and CLI authority and returns on the ordered queue; a failed combined look/photo save retains the editor. Pending/unsaved Look intent participates in native close and quit admission. The renderer drives `nativeAvatarMotion` from shared frame time inside visible paint callbacks, reads OS Reduce Motion, requests another paint only when moving, and resets controllers on account/connection teardown. Window hide/minimize suppress motion. Activity-state selection, group clusters, occlusion proof and installed Windows/Linux acceptance remain integration gates; current chat portraits select idle.

Transcript and sidebar chat avatars use the bot's state in that chat; profiles and bot lists use the strongest state across its chats (`AppStore.avatarState`, `Model/BotActivity.swift`). Retries are keyed by chat and bot. Precedence: a newly observed error for eight seconds after a failed turn ends, active retry, a current-turn pending card or command question, a running tool, streaming text, thinking, else idle. A new user message bounds the current turn; historical cards and streaming rows do not carry into it. Error-expiry callbacks are cancelled on new turns, removal, reset, snapshot replacement, and disconnect.

The Look sheet (`Sheets/BotLookViewController.swift`, draft in `Sheets/BotLookDraft.swift`) edits the base appearance and each state's override: shape, expression, background, tone, hue, body/eye/background colors, and motion. It has a live preview, an activity cycle, contrast warnings, Shuffle, Use Every State, Restore Default, and the image. Explicit state choices remain overrides even when they equal the base, preserving intent across future base edits. Save sends changed look and photo fields together in one awaited `bots.update`; unchanged fields are omitted. While Save is pending, Save and Cancel are disabled and the sheet's dismissal policy blocks Cancel actions and Escape. A failure retains the entire draft, displays the error without dismissing, and enables Save and Cancel again.

## Protocol compatibility and rollout

Appearance schema version 1 remains independent of the fresh Beans format boundary. Protocol 5 and `Beans-Format: beans-v2` protect all account routes, including roster PUTs, before storage. Effective account and roster floors cannot fall below 5; health advertises format and both floors. Missing, malformed, duplicated or unsupported capability evidence fails closed before sync. Old-format clients receive `426`, not a lower-floor route exception. Saved look, photo precedence, roster reconciliation and renderers are unchanged.

Individual `DELETE /v1/blobs/{id}` cannot remove roster or policy blobs, regardless of the admitted client's protocol. Both SQLite and Postgres exclude these kinds inside the atomic DELETE statement; protected ids answer `404` without changing their rows, identity sequence, or usage. Photo files and consumed envelopes retain their existing deletion path. Roster replacement uses its conditional slot write instead.

Cross-Device customization uses protocol-5/`beans-v2` peers on every participating replica and Device. This is an incompatible fresh account format with no automatic migration or reuse of an old account. Existing accounts and production services remain untouched; source/local receipts do not authorize deployment. Any separately authorized rollout or rollback must preserve appearance and format/floor enforcement.

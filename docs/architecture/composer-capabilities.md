# Composer capability dispositions

The [shared composer contract](composer.md) defines selection and authority. These are proposed consumers, not implemented controls.
Each row selects a reuse-backed future consumer or explicit discovery dependency, not a screenshot service or fake enabled control.

| Capability | Selected behavior / bounded dependency |
| --- | --- |
| Files | Reuse current explicit file attachments and preview chips. Apply existing size/count limits and Device-local picker permissions; selection alone never sends. |
| Folders | Discovery required: secure Runner-root selection, bounded enumeration, symlink/mount policy, byte/count limits, preview inventory and explicit approval. Current AppKit file panel rejects directories; never silently zip or traverse one. Depends on a folder-context contract, not skill discovery labels. |
| Terminal attachment | [Terminal sessions](terminal-sessions.md) supply sessions/output, not composer attachments. Discover exact bot/Runner/session ownership, immutable bounded output snapshot, redaction and revision. Never attach a live terminal handle, environment or other bot's output. |
| Project | Discover stable project identity, Runner/root authority and multi-bot sharing policy. A working directory is not a project ID or permission; no implicit repository/home scan. |
| Goal | Discover a durable bot/chat-owned goal model and lifecycle before offering a goal chip. Ordinary prompt text remains text, not a silently persisted goal. |
| Plan mode | Discover an enforceable per-turn read-only/tool policy and scope before a mode control. A planning instruction or saved workflow is not execution prevention. |
| Record skill | Discovery required for capture source, explicit consent, secret scrubbing, preview, provenance/license and activation boundary. Existing managed copies remain inactive; recording never auto-imports or enables them. |
| Sketch | Existing image attachment can carry an explicitly saved sketch. A drawing editor/canvas, export format and accessible alternative require separate native-client discovery; no screenshot asset reuse. |
| Plugins | Reuse installed per-Runner state, bot capability filtering and separate setup/auth sheets from [Plugins](plugins.md). A catalog pick does not install, sign in, connect or invoke. Plugin skill activation and stable installed-instance IDs remain dependencies. |
| Chats | Reuse authorized local `chats.search` discovery coverage, but typed cross-chat context needs explicit target membership, bounded preview and stable message/chat revisions. No provider-private history access or whole-history export. |
| Artifacts | Reuse attachments, draft proposals and the parser/private store in the [current inventory](composer.md#current-inventory-and-reusable-catalogs). Complete their listed production consumers plus owned access, approved-versus-pending state and bounded previews before context picks. No fake registry or duplicate identity scheme. |
| Code review | Reuse reviewed tool/workflow mechanisms only after discovering an immutable diff/base/head snapshot, repository permission, explicit target and review action schema. Auto-review of tool effects is not a user-facing code-review command. |
| Speed | No generic speed capability established. Discover meaningful provider-supported latency/quality controls and cost disclosure; do not infer a “fast” service from the screenshot. |
| Model | Existing bot model/provider/thinking settings and [model catalog](model-catalog.md) remain explicit bot edits. A per-turn override requires its own capability and persistence contract; picker selection must not silently change the saved model. |
| Memory | Reuse [memory service UI](memory-ui.md) scoped status and explicit panels. Composer context requires exact bot/connection IDs, permission, bounded preview and revision; no search/capture/delete on mount or selection. |
| MCP status | Reuse selected Runner server health/state from [MCP servers](mcp-servers.md). Status is read-only and distinguishes offline, connecting, auth/setup and error. Reconnect/sign-in/tool invocation remain explicit operations outside selection. |

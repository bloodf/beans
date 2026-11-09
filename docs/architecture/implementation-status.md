# Implementation status

## Status

Implemented mechanisms include crypto and blob sync, relay storage, CLI identity/pairing/restore, the local websocket, API-key and ChatGPT subscription providers, server-side web search, the agent loop, group chats, cross-Runner jobs and handoffs, steering/stop, routines, MCP plugins, permission cards, encrypted phone pushes, `beans service`, and signed app/server update readiness with Runner admission drain. Standalone CLI self-update is unavailable; production Grok OAuth is disabled pending a verified Beans contract. Source implementation is not installed-app or live-service acceptance.

Next: keychain storage, a cost budget per chat.

See [Phone app](phone-app.md) for phone identity and key ownership.

## Open points

- Keychain instead of 0600 files for the master secret and credentials

When those are chosen, update this file.

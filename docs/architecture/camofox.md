# Camofox evaluation

Camofox ([`jo-inc/camofox-browser`](https://github.com/jo-inc/camofox-browser)) is a REST service that drives a Camoufox (patched Firefox) browser, plus an stdio MCP adapter that forwards to it. This doc records the pin, provenance, license, bind/auth, cookie, telemetry and lifecycle facts read from public source for issue #77. Beans has no Camofox integration: no Runner starts it, no marketplace entry names it, and the Browser plugin stays the pinned `@playwright/mcp@0.0.83` (`crates/cli/marketplace/index.json`, id `playwright`).

## Disposition

**No-go until the gates below close.** The source revision and adapter version are pinned, but bind/auth defaults, telemetry defaults, browser-binary pinning and binary redistribution licensing are unresolved. Camofox issues #78-#84 stay blocked. Evidence below is source-read and registry metadata only; nothing was installed, executed or probed live. Live behavior is pending evidence.

## Pin

| Item | Value | Evidence |
| --- | --- | --- |
| Source revision | `39c82094013480b373df6600d44c7f036f58356e`, tag `v1.18.1`, `master` head when read (2026-10-13), committed 2026-10-05 | `gh api repos/jo-inc/camofox-browser/commits/master --jq .sha`; `gh api repos/jo-inc/camofox-browser/tags` |
| Adapter | `@askjo/camofox-browser-mcp@1.18.1`, MIT, Node >=22, one dependency `@modelcontextprotocol/sdk ^1.30.0`, published 2026-10-05 | [Versioned registry metadata](https://registry.npmjs.org/@askjo%2fcamofox-browser-mcp/1.18.1); `npm view @askjo/camofox-browser-mcp@1.18.1 --json` |
| Adapter `gitHead` | equals the revision above | same command, `gitHead` |
| Tarball integrity | `sha512-ZeJTiaY+8WMZWPBuPlPJR7U5tNz87Ed2YYc7lm/a3oX/sqpK3h+BlQmHBfbXLOT9TUTvMcgvH6wbJF0TlSNzjQ==` | `npm view @askjo/camofox-browser-mcp@1.18.1 dist.integrity` |
| Core server | `package.json` at the revision pins `camoufox-js` 0.11.5, `playwright-core ^1.58.0` (range, not exact), `express ^5.2.1`, `better-sqlite3` 13.0.1 | [`package.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/package.json) |

The core server is not published to npm by this evaluation; only the adapter is. Use the revision above for any source read, never `master`.

## Adapter tools

The adapter is stdio only and forwards to a REST service at `CAMOFOX_BASE_URL` (default `http://localhost:9377`). It does not need the browser, Playwright or the core server's dependencies ([`mcp/package.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/package.json), [`mcp/README.md`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/README.md)). `TOOL_DEFS` in [`mcp/lib/tool-contracts.mjs`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/tool-contracts.mjs) lists exactly 11 tools:

`camofox_create_tab`, `camofox_snapshot`, `camofox_click`, `camofox_type`, `camofox_navigate`, `camofox_scroll`, `camofox_screenshot`, `camofox_close_tab`, `camofox_evaluate`, `camofox_list_tabs`, `camofox_import_cookies`.

`camofox_evaluate` runs arbitrary JavaScript in the page; `camofox_import_cookies` reads a file from the Runner's disk and loads it into the session. Neither is read-only, so both would need Auto-review and must never be marked read-only in a manifest.

Reproduce: `curl -s https://raw.githubusercontent.com/jo-inc/camofox-browser/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/tool-contracts.mjs | grep -o "name: 'camofox_[a-z_]*'"`.

## Bind and auth

Source facts from [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js), [`lib/auth.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/auth.js) and [`server.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/server.js):

- `app.listen(PORT, CONFIG.bindHost || undefined, …)`. `CAMOFOX_BIND_HOST` is empty by default, so Node binds all interfaces. The project README says the same ("bound to all interfaces by default"). Loopback-only binding is available only by setting `CAMOFOX_BIND_HOST=127.0.0.1`; nothing enforces it.
- `accessKeyMiddleware` enforces `CAMOFOX_ACCESS_KEY` with a timing-safe Bearer comparison when the key is set. It exempts `/health` regardless of method, `POST` cookie import and `/auth-sessions/*` when `CAMOFOX_API_KEY` is set, and `POST /stop` when `CAMOFOX_ADMIN_KEY` is set. When the access key is unset it calls `next()`: ordinary browsing routes, including tab creation and `evaluate`, are unauthenticated, while cookie import, auth-session and admin routes retain their independent gates.
- Cookie import checks `CAMOFOX_API_KEY` explicitly in `server.js`; `CAMOFOX_ACCESS_KEY` does not substitute for it. Without the API key, import is allowed only from loopback addresses and only when `NODE_ENV` is not `production`.
- `POST /stop` checks `CAMOFOX_ADMIN_KEY` through the `x-admin-key` header.
- The adapter sends `CAMOFOX_ACCESS_KEY` (and `CAMOFOX_API_KEY` for cookie import) from its own environment and does not verify that the server enforces either.

This is a separate trust boundary from Beans. The Beans CLI binds `127.0.0.1` and applies its host and origin checks only to its own `/ws` socket ([Local API](local-api.md)); those checks do not cover an external Camofox server, which enforces only the keys above.

A usable Beans contract would need all three: loopback or private-socket bind, a generated per-Runner `CAMOFOX_ACCESS_KEY`, and a startup check that an unauthenticated request is refused. None is verified live. The unauthenticated default is the blocking finding.

Reproduce: `grep -n "bindHost\|accessKey" lib/config.js`; `sed -n 100,135p lib/auth.js`; `grep -n "app.listen" server.js` at the revision.

## Cookie path containment

`readCookieFile` in [`mcp/lib/cookies.mjs`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/cookies.mjs) rejects empty and absolute `cookiesPath`, resolves the cookies directory with `realpath`, requires the resolved target to be inside it, resolves the target with `realpath` again and re-checks containment (so a symlink out is rejected), and caps the file at 5 MiB. `cookiesDir` defaults to `~/.camofox/cookies` (`CAMOFOX_COOKIES_DIR`). `domainSuffix` filters by `endsWith`. This is source-read containment, covered by upstream tests not run here. The REST body limit for `POST /sessions/:userId/cookies` is 512 kB.

## Browser binary provenance

The adapter ships no browser. The core server downloads Camoufox:

- [`Dockerfile`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/Dockerfile) pins `CAMOUFOX_VERSION=152.0.4`, `CAMOUFOX_RELEASE=beta.28` and fetches the zip from `github.com/daijro/camoufox/releases` with `curl -fL`, with no checksum check.
- [`lib/camoufox-download.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/camoufox-download.js) (`npm run fetch-bin`) pins `152.0.4-beta.30` and also downloads GeoIP data and the default add-ons through `camoufox-js`.

The two paths disagree (beta.28 versus beta.30). The Dockerfile does not check a digest. The JavaScript wrapper has no explicit digest check and delegates `fetcher.install()` to `camoufox-js`; the delegated verifier remains unverified. GitHub release metadata does publish SHA-256 digests (beta.30 `lin.x86_64`: `5720d45b894ce1770543de024c6f10d514b38be560fa2dc3226b3d8586caf672`; `mac.arm64`: `3b43e766574f286a6a63296cf58b660b7a3120952086c869b4df4c9a71604bc3`), so a Beans pin could verify them. Release tags `v152.0.4-beta.28` and `v152.0.4-beta.30` of [`daijro/camoufox`](https://github.com/daijro/camoufox) resolve to commits `0583c3ec94f5a9df5cb2d09553fbfe80589b6e2d` and `5d06ec1629ac7843508f1e683f83e404fde8db76`. Native platforms published for each: Linux x86_64/arm64, macOS x86_64/arm64, Windows x86_64/i686. The Dockerfile maps only Linux amd64/arm64.

Reproduce: `gh api repos/daijro/camoufox/releases/tags/v152.0.4-beta.30 --jq '[.assets[]|.name+" "+.digest]'`.

## Licenses

| Component | License | Source |
| --- | --- | --- |
| Camofox wrapper and adapter | MIT | [LICENSE](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/LICENSE) |
| `camoufox-js` 0.11.5 | MPL-2.0 | `npm view camoufox-js@0.11.5 license`; [repo](https://github.com/apify/camoufox-js) |
| `playwright-core` 1.62.1 (core lockfile) | Apache-2.0 | [Versioned registry metadata](https://registry.npmjs.org/playwright-core/1.62.1); [`package-lock.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/package-lock.json); `npm view playwright-core@1.62.1 license` |
| Camoufox browser source (patches, build) | MPL-2.0 | [Pinned LICENSE](https://github.com/daijro/camoufox/blob/5d06ec1629ac7843508f1e683f83e404fde8db76/LICENSE) |
| Browser-bundled third-party code, including any trajectory implementation | license inventory not reviewed at the selected revision | pending |
| Default browser add-ons, including uBlock Origin | upstream licenses and artifact provenance not reviewed here | pending |
| Firefox binary components and bundled third-party code | not enumerated; the wrapper's MIT license does not cover them | pending |
| Other transitive dependencies of `camoufox-js` and the core server | not enumerated | `package-lock.json` at the revision, sha `eb7fda83160fda2dfe29b70c39d211bcfb7f9eb1` |

Unresolved: whether Beans may redistribute or auto-download the Camoufox zip, the default add-ons and the GeoIP data, and what notices a Beans bundle must carry. The choice between fetching from upstream and bundling remains pending.

## Telemetry

Crash and hang telemetry is on by default in [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js): `crashReportEnabled: process.env.CAMOFOX_CRASH_REPORT_ENABLED !== 'false'`. The README describes anonymized reports posted to a Cloudflare Worker (`camofox-telemetry.askjo.workers.dev`) that files GitHub issues, and says opt-out is `CAMOFOX_CRASH_REPORT_ENABLED=false`. Sentry reporting is separate and active only when `SENTRY_DSN` is set. A Beans deployment would have to set the opt-out and leave `SENTRY_DSN` unset; the reporter source (`lib/reporter.js`) was not audited line by line, so the anonymization claim is upstream-documented, not verified.

## Persistence, expiry, cancellation

Defaults below are read from the pinned [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js).

- Cookies and localStorage persist per user in `~/.camofox/profiles/` by default (`CAMOFOX_PROFILE_DIR`); `camofox.config.json` can disable persistence. IndexedDB persistence is opt-in.
- Session timeout defaults to 600000 ms (`SESSION_TIMEOUT_MS`), tab inactivity 300000 ms, 10 tabs per session, 50 globally.
- The adapter exposes `camofox_close_tab` but no abort for an in-flight call. Cancellation of a running page action is not shown in source read and stays unverified.

## Open gates

1. Auth: an unauthenticated start is the default; a Beans-side enforced key and loopback bind must be shown live.
2. Binary: pick one release, verify its SHA-256 on download, reconcile the beta.28/beta.30 disagreement.
3. Licenses: review the Firefox, add-on, GeoIP and transitive dependency licenses and the redistribution terms.
4. Telemetry: confirm the opt-out disables all outbound reports on a live run.
5. Cancellation and cookie containment: confirm on a live run.

Closing the five gates is a precondition for #78-#84. Until then Beans keeps the pinned Playwright Browser plugin unchanged.

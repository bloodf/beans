# Camofox evaluation

Camofox ([`jo-inc/camofox-browser`](https://github.com/jo-inc/camofox-browser)) is a REST service that drives a Camoufox (patched Firefox) browser, plus an stdio MCP adapter that forwards to it. This doc records the pin, provenance, license, bind/auth, cookie, telemetry and lifecycle facts read from public source for issue #77. Beans has no Camofox integration: no Runner starts it, no marketplace entry names it, and the Browser plugin stays the pinned `@playwright/mcp@0.0.83` (`crates/cli/marketplace/index.json`, id `playwright`).

## Disposition

**No-go until the gates below close.** The source revision and adapter version are pinned, but bind/auth defaults, telemetry defaults, browser-binary pinning and binary redistribution licensing are unresolved. Camofox issues #78-#84 stay blocked. Evidence below is source-read and registry metadata only; nothing was installed, executed or probed live. Live behavior is pending evidence.

## Pin

| Item | Value | Evidence |
| --- | --- | --- |
| Source revision | `39c82094013480b373df6600d44c7f036f58356e`; annotated tag `v1.18.1` peels to this commit | `git ls-remote https://github.com/jo-inc/camofox-browser.git 'refs/tags/v1.18.1*'`; versioned adapter registry `gitHead` independently matches |
| Adapter | `@askjo/camofox-browser-mcp@1.18.1`, MIT, Node >=22, one dependency `@modelcontextprotocol/sdk ^1.30.0` | [Versioned registry metadata](https://registry.npmjs.org/@askjo%2fcamofox-browser-mcp/1.18.1) and pinned `mcp/package.json` |
| Adapter `gitHead` | equals the revision above | versioned registry metadata |
| Tarball integrity | `sha512-ZeJTiaY+8WMZWPBuPlPJR7U5tNz87Ed2YYc7lm/a3oX/sqpK3h+BlQmHBfbXLOT9TUTvMcgvH6wbJF0TlSNzjQ==` | versioned registry `dist.integrity`; tarball bytes were not downloaded or verified |
| Core server | `package.json` at the revision pins `camoufox-js` 0.11.5, `playwright-core ^1.58.0` (range, not exact), `express ^5.2.1`, `better-sqlite3` 13.0.1 | [`package.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/package.json) |

This evaluation pins the published adapter and core source separately; it does not establish whether the core package is published. Use the revision above for source reads, never `master`. Git tag references and registry/release metadata can change; immutable commit URLs anchor the source findings. Metadata integrity and release digests are recorded values, not verification of downloaded artifacts.

## Adapter tools

The adapter is stdio only and forwards to a REST service at `CAMOFOX_BASE_URL` (default `http://localhost:9377`). It does not need the browser, Playwright or the core server's dependencies ([`mcp/package.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/package.json), [`mcp/README.md`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/README.md)). `TOOL_DEFS` in [`mcp/lib/tool-contracts.mjs`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/tool-contracts.mjs) lists exactly 11 tools:

`camofox_create_tab`, `camofox_snapshot`, `camofox_click`, `camofox_type`, `camofox_navigate`, `camofox_scroll`, `camofox_screenshot`, `camofox_close_tab`, `camofox_evaluate`, `camofox_list_tabs`, `camofox_import_cookies`.

`camofox_evaluate` runs arbitrary JavaScript in the page; `camofox_import_cookies` reads a file from the Runner's disk and loads it into the session. Neither is read-only, so both would need Auto-review and must never be marked read-only in a manifest.

Reproduce without GitHub API: fetch the [raw pinned tool contracts](https://raw.githubusercontent.com/jo-inc/camofox-browser/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/tool-contracts.mjs) and inspect `TOOL_DEFS`. The pinned [adapter entrypoint](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/server.mjs) returns that list in its MCP list-tools handler; no server is launched by this check.

## Bind and auth

Source facts from [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js), [`lib/auth.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/auth.js) and [`server.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/server.js):

- `app.listen(PORT, CONFIG.bindHost || undefined, …)`. `CAMOFOX_BIND_HOST` is empty by default, so Node binds all interfaces. The project README says the same ("bound to all interfaces by default"). Loopback-only binding is available only by setting `CAMOFOX_BIND_HOST=127.0.0.1`; nothing enforces it.
- `accessKeyMiddleware` enforces `CAMOFOX_ACCESS_KEY` with a timing-safe Bearer comparison when the key is set. It exempts `/health` regardless of method, `POST` cookie import and `/auth-sessions/*` when `CAMOFOX_API_KEY` is set, and `POST /stop` when `CAMOFOX_ADMIN_KEY` is set. When the access key is unset it calls `next()`: ordinary browsing routes, including tab creation and `evaluate`, are unauthenticated, while cookie import, auth-session and admin routes retain their independent gates.
- Cookie import checks `CAMOFOX_API_KEY` explicitly in `server.js`; `CAMOFOX_ACCESS_KEY` does not substitute for it. Without the API key, import is allowed only from loopback addresses and only when `NODE_ENV` is not `production`.
- `POST /stop` checks `CAMOFOX_ADMIN_KEY` through the `x-admin-key` header.
- The adapter sends `CAMOFOX_ACCESS_KEY` (and `CAMOFOX_API_KEY` for cookie import) from its own environment and does not verify that the server enforces either.

This is a separate trust boundary from Beans. The Beans CLI binds `127.0.0.1` and applies its host and origin checks only to its own `/ws` socket ([Local API](local-api.md)); those checks do not cover an external Camofox server, which enforces only the keys above.

A usable Beans contract would need all three: loopback or private-socket bind, a generated per-Runner `CAMOFOX_ACCESS_KEY`, and a startup check that an unauthenticated request is refused. None is verified live. The unauthenticated default is the blocking finding.

Reproduce from the raw files at the revision: `lib/config.js` defines empty `bindHost` and `accessKey`; `lib/auth.js` defines `accessKeyMiddleware`; `server.js` mounts it before the routes and calls `app.listen`. Cookie import has its own inline API-key check, not the shared `requireAuth` helper (which accepts the access key as an alternative on other routes).

## Cookie path containment

`readCookieFile` in [`mcp/lib/cookies.mjs`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/lib/cookies.mjs) rejects empty and absolute `cookiesPath`, resolves the cookies directory with `realpath`, checks lexical containment, resolves the target with `realpath` and re-checks containment (so a static symlink out is rejected), then checks file size and reads it. The limit is 5 MiB; `cookiesDir` defaults to `~/.camofox/cookies` (`CAMOFOX_COOKIES_DIR`). `domainSuffix` filters by `endsWith`, not a domain-label boundary. This is source-read containment, not race-safe filesystem isolation: `realpath`, `stat` and `readFile` are separate operations without a retained file handle, so concurrent path replacement remains unresolved. No upstream tests were run here. The REST route declares a 512 kB parser, but the earlier global 100 kB parser also matches cookie import; 512 kB is not an established effective limit. The adapter parses the local file and sends cookie objects, never a filesystem path.

## Browser binary provenance

The adapter ships no browser. The core server downloads Camoufox:

- [`Dockerfile`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/Dockerfile) pins `CAMOUFOX_VERSION=152.0.4`, `CAMOUFOX_RELEASE=beta.28` and fetches the zip from `github.com/daijro/camoufox/releases` with `curl -fL`, with no checksum check.
- [`lib/camoufox-download.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/camoufox-download.js) (`npm run fetch-bin`) pins `152.0.4-beta.30` and also downloads GeoIP data and the default add-ons through `camoufox-js`.

The two paths disagree (beta.28 versus beta.30). The Dockerfile does not check a digest. The JavaScript wrapper has no explicit digest check and delegates `fetcher.install()` to `camoufox-js`; the delegated verifier remains unverified. The [beta.30 release asset page](https://github.com/daijro/camoufox/releases/expanded_assets/v152.0.4-beta.30) advertises SHA-256 digests: `lin.x86_64` is `5720d45b894ce1770543de024c6f10d514b38be560fa2dc3226b3d8586caf672`; `mac.arm64` is `3b43e766574f286a6a63296cf58b660b7a3120952086c869b4df4c9a71604bc3`. These are upstream metadata, not a selected Beans binary pin or hashes measured from downloaded bytes. Release tags `v152.0.4-beta.28` and `v152.0.4-beta.30` resolve through Git transport to commits `0583c3ec94f5a9df5cb2d09553fbfe80589b6e2d` and `5d06ec1629ac7843508f1e683f83e404fde8db76`. Asset pages for [beta.28](https://github.com/daijro/camoufox/releases/expanded_assets/v152.0.4-beta.28) and beta.30 list Linux x86_64/arm64, macOS x86_64/arm64 and Windows x86_64/i686. This proves advertised availability, not native execution. The Dockerfile maps only Linux amd64/arm64; it is provenance evidence, not a Beans provisioning design.

Reproduce without GitHub API: `git ls-remote https://github.com/daijro/camoufox.git 'refs/tags/v152.0.4-beta.28*' 'refs/tags/v152.0.4-beta.30*'`, then read the linked release asset pages. No browser artifact is downloaded.

## Licenses

| Component | License | Source |
| --- | --- | --- |
| Camofox wrapper and adapter | MIT | [LICENSE](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/LICENSE) |
| `camoufox-js` 0.11.5 | MPL-2.0 declaration; packaged license text not verified | [Versioned registry metadata](https://registry.npmjs.org/camoufox-js/0.11.5) and pinned core lockfile |
| `playwright-core` 1.62.1 (core lockfile) | Apache-2.0 | [Versioned registry metadata](https://registry.npmjs.org/playwright-core/1.62.1); [`package-lock.json`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/package-lock.json); [license at registry `gitHead`](https://github.com/microsoft/playwright/blob/26a9e470a7b3c7822084b09fb7f13902c5f37b51/LICENSE) |
| Camoufox browser source (patches, build) | MPL-2.0 | [Pinned LICENSE](https://github.com/daijro/camoufox/blob/5d06ec1629ac7843508f1e683f83e404fde8db76/LICENSE) |
| `ua-parser-js` 2.0.10 (transitive through `camoufox-js`) | AGPL-3.0-or-later declaration | [Versioned registry metadata](https://registry.npmjs.org/ua-parser-js/2.0.10); [license at registry `gitHead`](https://github.com/faisalman/ua-parser-js/blob/4121c59060c3f9b814f07978c361e44016028c74/LICENSE.md); pinned core lockfile |
| `@modelcontextprotocol/sdk` 1.30.0 (core lockfile; adapter requests a range) | MIT declaration; adapter resolution is not established by the core lockfile | pinned core lockfile and adapter manifest |
| Browser-bundled third-party code, including any trajectory implementation | license inventory not reviewed at the selected revision | pending |
| Default browser add-ons, including uBlock Origin | upstream licenses and artifact provenance not reviewed here | pending |
| Firefox binary components and bundled third-party code | not enumerated; the wrapper's MIT license does not cover them | pending |
| Remaining transitive dependencies of the adapter, `camoufox-js` and core server | complete artifact/license inventory and notices not reviewed | [Pinned core lockfile](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/package-lock.json) and [adapter lockfile](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/mcp/package-lock.json) are inventory inputs, not a completed license audit |

Unresolved: whether Beans may redistribute or auto-download the Camoufox zip, the default add-ons and the GeoIP data, and what notices a Beans bundle must carry. The choice between fetching from upstream and bundling remains pending.

Known obligations are not a redistribution approval: MIT requires its copyright and permission notice; MPL-2.0 sections 3.1–3.4 require covered-source availability and notices when distributing executables; Apache-2.0 section 4 requires the license, retained notices and applicable NOTICE attribution. The transitive AGPL declaration needs a compatibility and corresponding-source assessment for the actual distribution and any modified network service; wrapper MIT does not settle it. The browser source license alone does not enumerate the zip's Firefox components, add-ons or GeoIP terms. Those artifact-level obligations and the full dependency inventory remain blocking, not assumed permissive.

## Telemetry

Crash and hang telemetry is on by default in [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js): `crashReportEnabled: process.env.CAMOFOX_CRASH_REPORT_ENABLED !== 'false'`. [`lib/reporter.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/reporter.js) posts reports to a Cloudflare Worker and returns disabled reporting/watchdog functions when `crashReportEnabled` is false. Its anonymization is not a verified privacy guarantee. Sentry is a separate path; its configuration defaults to an empty `SENTRY_DSN`. A Beans deployment would have to set `CAMOFOX_CRASH_REPORT_ENABLED=false` and leave `SENTRY_DSN` unset. Source inspection of the disabled reporter branch does not establish absence of all outbound telemetry or browser network traffic; that remains a separately authorized runtime gate.

## Persistence, expiry, cancellation

Defaults below are read from the pinned [`lib/config.js`](https://github.com/jo-inc/camofox-browser/blob/39c82094013480b373df6600d44c7f036f58356e/lib/config.js).

- Cookies and localStorage persist per user in `~/.camofox/profiles/` by default (`CAMOFOX_PROFILE_DIR`); `camofox.config.json` can disable persistence. IndexedDB persistence is opt-in.
- Session timeout defaults to 600000 ms (`SESSION_TIMEOUT_MS`), tab inactivity 300000 ms, 10 tabs per session, 50 globally.
- The adapter exposes `camofox_close_tab` but no abort for an in-flight call. Cancellation of a running page action is not shown in source read and stays unverified.

## Open gates

1. Auth: an unauthenticated start is the default; a Beans-side enforced key and loopback bind must be shown live.
2. Binary: pick one release, verify its SHA-256 on download, reconcile the beta.28/beta.30 disagreement.
3. Licenses: complete the artifact and transitive dependency inventory, including AGPL compatibility, Firefox, add-on and GeoIP terms, source availability and bundled notices.
4. Telemetry: confirm the opt-out disables all outbound reports on a live run.
5. Cancellation and cookie containment: establish cancellation and concurrent path-replacement containment, then confirm on a separately authorized live run.

Closing the five gates is a precondition for #78-#84. Until then Beans keeps the pinned Playwright Browser plugin unchanged.

## Discovery evidence boundary

The revision, registry metadata, tool definitions, configuration, auth routes, cookie reader, binary download paths, source licenses and release asset listings above are re-read on 2026-10-14 using public raw-source/registry reads and `git ls-remote`, without GitHub API. No installation, browser/VM probe, provisioning or runtime acceptance occurs. A no-go discovery disposition is distinct from a usable security contract; unresolved license inventory and runtime gates remain explicit. Downstream #78–#84 must retain the no-go outcome before any install work; publishing those issue updates is separate from this document.

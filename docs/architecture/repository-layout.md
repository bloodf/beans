# Repository layout

## Repo layout

```
beans/
  ARCHITECTURE.md      # this overview and the list of subjects
  README.md
  docs/architecture/   # one doc per subject
  docs/agent/          # beans-agent's own documentation
  Cargo.toml           # workspace
  crates/agent/        # beans-agent: loop, tools, codemode (QuickJS), and Messages, Chat Completions, Responses, ChatGPT, and Grok providers
  crates/models/       # beans-models: the bundled model catalog (windows, thinking levels, rates); Devices check the selected Beans relay's public feed for updates
  crates/provider-auth/ # OAuth token types and PKCE flows shared by every Device
  crates/tls/          # beans-tls: the certificate trust of every Device's HTTPS, the system's on macOS and Windows
  crates/cli/          # beans: the Device core as a library (keys, relay sync, jobs, the JSON API) + runner and server features + the binary
  crates/mobile/       # beans-mobile: the core for the phone over UniFFI
  crates/markdown/     # beans-markdown: message Markdown as the blocks and spans every app renders (pulldown-cmark, and GitHub's autolinks for bare URLs and addresses), for the Mac and phone over UniFFI
  crates/relay/        # beans-relay: axum + SQLite or Postgres, and its Dockerfile
  macos/               # AppKit SPM app; the build bundles the CLI
  desktop/             # the Windows and Linux app: MyGo (Go + system webview) with a Solid page; the build bundles the CLI
  mobile/              # Expo app for iOS and Android: a paired Device over the core (modules/beans-core)
  web/                 # the site
  scripts/             # Node/TypeScript scripts: dev loop, bundle build, macOS release, the desktop app's dev loop and builds, string and doc checks
  .github/workflows/   # release.yml: manual signed Beans releases; test.yml: full PR/manual test matrix; docs.yml: PR/manual doc check; catalog.yml: PR/manual catalog/index check
```

See [Phone development](phone-app.md#development-loop) for phone rebuild commands.

`pnpm run dev` runs the macOS Beans Dev loop; `pnpm run build` produces the Beans release bundle. See [macOS development and builds](macos-app.md#development-and-builds) for rebuild, launch and SDK-stamping mechanisms. `pnpm run relay` runs a local relay. The [phone development loop](phone-app.md#development-loop) fingerprints native inputs, rebuilds stale pieces and leaves production Beans processes alone.

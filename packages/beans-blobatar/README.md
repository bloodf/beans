# Beans Blobatar

Project-owned, offline avatar generator. Vendored from [Alain00/blobatar](https://github.com/Alain00/blobatar) **2.7.0**, commit [`a7fd546ebede49d0a9fa638945b9e534489782a2`](https://github.com/Alain00/blobatar/commit/a7fd546ebede49d0a9fa638945b9e534489782a2). Source: `packages/blobatar/src` and `packages/react-native/src/index.tsx`, `parts.tsx`. Original copyright **Copyright (c) 2026 Alain**; [MIT license](LICENSE) retained verbatim. Core shape bands, layout, palette, hash, and serialization remain upstream code. Local edits: native imports point into vendored core; hash constructs `TextEncoder` lazily for JavaScriptCore; `jsc.ts` supplies standard UTF-8 encoding in hosts without `TextEncoder`.

## Seed contract

Use **bot ID** (`bot.id`), never bot name, device ID, chat ID, or runtime-specific hash. `botAvatarSeed(botID) === botID`. Upstream normalizes strings to NFC, trims, and lowercases by default. Within this pinned release, equal IDs and equal options produce equal SVG/geometry across Bun, Node, JavaScriptCore, and React Native. Changing upstream version can change looks; treat version bumps as visual migrations. Avatar rendering fetches nothing.

## Bun / Node / desktop Solid

```ts
import { blobatar, botAvatarSeed } from "@beans/blobatar";
const svg: string = blobatar(bot.id); // complete, deterministic inline SVG
// Solid: <div innerHTML={blobatar(bot.id)} /> only for trusted bot IDs
// For <img>, encode locally: `data:image/svg+xml,${encodeURIComponent(svg)}`.
```

`index.js` is committed ESM built from `index.ts` and the vendored core; Node imports it without TypeScript tooling. Import package using local file dependency (`"@beans/blobatar": "file:../packages/beans-blobatar"`, relative to consuming app) or `../../packages/beans-blobatar/index.js`. Do not render network URLs. `blobatar(botID, options?)` accepts upstream [`BlobatarOptions`](vendor/core/render.ts): e.g. `{ background: "circle", size: 48 }`. Default viewBox is `0 0 100 100`; size omitted for responsive SVG.

## macOS AppKit / JavaScriptCore

Bundle committed `dist/blobatar.jsc.js` as app resource and evaluate once in a private `JSContext`. Script installs global `blobatar(botID, options?)`; pass bot ID as argument, not interpolated into JavaScript source. Take returned SVG string, then use `NSImage(data: Data(svg.utf8))` (existing PluginLogo SVG loading pattern). Cache output by bot ID if displayed in lists. No network, Bun, Node, DOM, or runtime bundle step required. JavaScriptCore's missing `TextEncoder` receives local UTF-8 implementation in this bundle; original upstream hash still consumes UTF-8 bytes.

```swift
context.evaluateScript(try String(contentsOf: resourceURL, encoding: .utf8))
let svg = context.objectForKeyedSubscript("blobatar")?.call(withArguments: [botID])?.toString()
let image = svg.flatMap { NSImage(data: Data($0.utf8)) }
```

## Expo / React Native

```tsx
import { Blobatar } from "@beans/blobatar/react-native";
<Blobatar name={bot.id} size={48} title={`Avatar of ${bot.name}`} />
```

Install `react-native-svg` in app (Expo: `npx expo install react-native-svg`). Native adapter uses real `<Svg>`, `<Path>`, `<Circle>`, `<G>` elements from `react-native-svg` and upstream `_marks` for same geometry; no remote image or XML parsing. `size` required. Without `title`, image is decorative and hidden from screen readers. Static `Blobatar`, `MorphingBlobatar` available; animation needs caller reduced-motion handling (choose static renderer when enabled). Metro resolves TSX source through package `react-native` export condition. No `react-native-reanimated` required for static renderer.

## Regenerating committed JavaScript

From repository root, use `Bun.build({ entrypoints: ["packages/beans-blobatar/index.ts"], target: "browser", format: "esm" })` for `index.js`; use `jsc.ts` with `format: "iife"` for `dist/blobatar.jsc.js`. Keep pinned upstream SHA, license, and core/native source in sync when upgrading. The committed bundles are source-derived; consumers never compile the JSCore bundle on device.

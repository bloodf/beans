# Beans Blobatar

Project-owned, offline avatar generator. Vendored from [Alain00/blobatar](https://github.com/Alain00/blobatar) **2.7.0**, commit [`a7fd546ebede49d0a9fa638945b9e534489782a2`](https://github.com/Alain00/blobatar/commit/a7fd546ebede49d0a9fa638945b9e534489782a2). Source: `packages/blobatar/src` (including `idle.ts`) and `packages/react-native/src/index.tsx`, `parts.tsx`, `animated.tsx`, `worklets.ts`; `test/golden/` is upstream's golden corpus. Original copyright **Copyright (c) 2026 Alain**; [MIT license](LICENSE) retained verbatim. Core shape bands, layout, palette, hash, serialization and idle loops remain upstream code. Local edits: imports point into the vendored tree; hash constructs `TextEncoder` lazily for JavaScriptCore; `jsc.ts` supplies standard UTF-8 encoding in hosts without `TextEncoder`.

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

## Appearance, geometry and animation

`appearance.ts` holds the `BotLook` schema (version 1): a base appearance and optional per-state overrides for `idle`, `thinking`, `responding`, `working`, `waiting`, `retry`, `error`. `validateBotLook` rejects unknown fields, unknown enum values, hue outside `[0, 360)` and any palette value that is not uppercase `#RRGGBB`. `resolveBotAppearance` merges a state over the base (palette channels individually); omitted shape, hue and tone keep the values seeded by the bot ID, a missing expression is `idle`, a missing background is `none` (transparent, as upstream draws it) and missing motion is on. Shape and tone pin the upstream `shape` trait and tone position inside their bands, so `botAvatarSVG(botID)` with no look is byte-identical to `blobatar(botID)`.

`botAvatarGeometry` returns the same figure as plain JSON numbers: the core silhouette normalized to 24 cubics (every source segment boundary kept, the rest split by de Casteljau at half the widest angular sweep about the body centre, clockwise from the top), the droplet taper as 4 cubics, nine petal slots (unused ones zero-radius at the body centre), the backdrop as 8 cubics with opacity 0 for `none`, the drawn eyes, the upstream pose, tinted fills and `motionSeeds`. `interpolateAvatarGeometry` lerps all of it, so shapes morph instead of cutting. `avatarFrame(g, timeMs, amp)` composes the upstream idle layer (breathe, bob, glance, blink, tremor, seesaw) into one body matrix and one matrix per eye; `amp` 0 stops every loop, including the tremor and seesaw expressions carry, and draws the static endpoint. `frame.ts` is all `"worklet"` functions, so a Reanimated UI-thread callback can call it; nothing parses SVG per frame. Caching endpoint geometry by bot ID and canonical look is the caller's.

`@beans/blobatar/react-native/animated` is upstream `AnimatedBlobatar`, driven by Reanimated 4 and `react-native-worklets`.

## Expo / React Native

```tsx
import { Blobatar } from "@beans/blobatar/react-native";
<Blobatar name={bot.id} size={48} title={`Avatar of ${bot.name}`} />
```

Install `react-native-svg` in app (Expo: `npx expo install react-native-svg`). Native adapter uses real `<Svg>`, `<Path>`, `<Circle>`, `<G>` elements from `react-native-svg` and upstream `_marks` for same geometry; no remote image or XML parsing. `size` required. Without `title`, image is decorative and hidden from screen readers. Static `Blobatar`, `MorphingBlobatar` available; animation needs caller reduced-motion handling (choose static renderer when enabled). Metro resolves TSX source through package `react-native` export condition. No `react-native-reanimated` required for static renderer.

## Regenerating committed JavaScript

Run `bun packages/beans-blobatar/build.ts` to regenerate `index.js` and `frame.js` (ESM) and `dist/blobatar.jsc.js` (IIFE that installs every export of `index.ts` as a global); two runs produce identical bytes. `bun test` in this directory runs the schema, geometry, morph, frame and upstream golden tests. Keep pinned upstream SHA, license, and core/native source in sync when upgrading. The committed bundles are source-derived; consumers never compile the JSCore bundle on device.

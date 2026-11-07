// Regenerates the committed bundles: `pnpm run build:blobatar`
// (tsx packages/beans-blobatar/build.ts).
//
// esbuild replaces Bun.build here with the same shape: one browser bundle per
// entry, ESM for the two committed modules and IIFE for the JavaScriptCore
// bundle, unminified and without source maps. Two runs produce identical bytes.
import { build, version } from "esbuild";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { runInNewContext } from "node:vm";

/** The pin lives in the root manifest; this build refuses any other version. */
const PINNED_ESBUILD = "0.28.2";
if (version !== PINNED_ESBUILD) {
  throw new Error(`esbuild ${version} is installed; this build is pinned to ${PINNED_ESBUILD}`);
}

const dir = import.meta.dirname;

/**
 * The globals the macOS app and the desktop native window read out of the
 * JavaScriptCore bundle. `macos/Sources/Beans/Design/AvatarGeometry.swift`
 * fails closed when one is missing; the build fails closed too.
 */
const JSC_GLOBALS = [
  "botAvatarGeometry",
  "interpolateAvatarGeometry",
  "avatarMorphProgress",
  "avatarFrame",
  "resolveBotAppearance",
  "botAppearanceContrast",
  "validateBotLook",
];

const bundles = [
  { entry: "index.ts", out: "index.js", format: "esm" },
  { entry: "frame.ts", out: "frame.js", format: "esm" },
  { entry: "jsc.ts", out: "dist/blobatar.jsc.js", format: "iife" },
] as const;

for (const { entry, out, format } of bundles) {
  const result = await build({
    entryPoints: [join(dir, entry)],
    outfile: join(dir, out),
    absWorkingDir: dir,
    bundle: true,
    write: false,
    platform: "browser",
    target: "esnext",
    format,
    minify: false,
    sourcemap: false,
    // The sources carry UTF-8 in comments; keep them as the bytes they are.
    charset: "utf8",
    legalComments: "none",
  });
  const output = result.outputFiles[0];
  if (!output) throw new Error(`build of ${entry} produced no output`);
  const path = join(dir, out);
  if (format === "iife") assertJSCGlobals(path, Buffer.from(output.contents).toString("utf8"));
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, output.contents);
}

/**
 * Evaluate the bundle the way JavaScriptCore does: on a fresh global without
 * `TextEncoder`, with the geometry globals installed on it. The check runs
 * before the file is written, so a bundle that breaks the contract is never
 * committed.
 */
function assertJSCGlobals(path: string, code: string): void {
  const sandbox: Record<string, unknown> = {};
  runInNewContext("delete globalThis.TextEncoder", sandbox);
  runInNewContext(code, sandbox);
  const missing = JSC_GLOBALS.filter((name) => typeof sandbox[name] !== "function");
  if (missing.length) throw new Error(`${path} does not install: ${missing.join(", ")}`);
  if (typeof sandbox.TextEncoder !== "function") {
    throw new Error(`${path} does not install a UTF-8 TextEncoder`);
  }
}

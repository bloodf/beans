import { build } from "esbuild";
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// Native bridge bundles the same source modules used by TS consumers.
for (const [entry, output] of [
  ["../packages/beans-blobatar/jsc.ts", "assets/blobatar.js"],
  ["src/model/nativeMemory.ts", "assets/memory.js"],
  ["src/model/nativeAvatarActivity.ts", "assets/avatar-activity.js"],
] as const) {
  const result = await build({ entryPoints: [fileURLToPath(new URL(entry, import.meta.url))], platform: "browser", target: "esnext", format: "iife", bundle: true, write: false, charset: "utf8" });
  const license = output === "assets/blobatar.js" ? `/*\n${await readFile(new URL("../packages/beans-blobatar/LICENSE", import.meta.url), "utf8")}\n*/\n` : "";
  const bundled = result.outputFiles[0];
  if (!bundled) throw new Error(`Native contract bundle missing: ${entry}`);
  await writeFile(new URL(output, import.meta.url), license + bundled.text);
}

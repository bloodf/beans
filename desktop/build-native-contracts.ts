// Native bridge bundles the same source modules used by TS consumers.
for (const [entry, output] of [
  ["../packages/beans-blobatar/jsc.ts", "assets/blobatar.js"],
  ["src/model/nativeMemory.ts", "assets/memory.js"],
  ["src/model/nativeAvatarActivity.ts", "assets/avatar-activity.js"],
] as const) {
  const result = await Bun.build({ entrypoints: [new URL(entry, import.meta.url).pathname], target: "browser", format: "iife" });
  if (!result.success) throw new AggregateError(result.logs, `Native contract bundle: ${entry}`);
  const license = output === "assets/blobatar.js" ? `/*\n${await Bun.file(new URL("../packages/beans-blobatar/LICENSE", import.meta.url)).text()}\n*/\n` : "";
  await Bun.write(new URL(output, import.meta.url), license + await result.outputs[0]!.text());
}

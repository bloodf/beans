// Native bridge bundles the same source modules used by TS consumers.
for (const [entry, output] of [
  ["../packages/beans-blobatar/jsc.ts", "assets/blobatar.js"],
  ["src/model/nativeMemory.ts", "assets/memory.js"],
] as const) {
  const result = await Bun.build({ entrypoints: [new URL(entry, import.meta.url).pathname], target: "browser", format: "iife" });
  if (!result.success) throw new AggregateError(result.logs, `Native contract bundle: ${entry}`);
  await Bun.write(new URL(output, import.meta.url), await result.outputs[0]!.text());
}

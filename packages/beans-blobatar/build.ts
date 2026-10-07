// Regenerates the committed bundles: `bun packages/beans-blobatar/build.ts`.
const dir = import.meta.dir;

for (const [entry, out, format] of [
  ["index.ts", "index.js", "esm"],
  ["frame.ts", "frame.js", "esm"],
  ["jsc.ts", "dist/blobatar.jsc.js", "iife"],
] as const) {
  const result = await Bun.build({ entrypoints: [`${dir}/${entry}`], target: "browser", format });
  if (!result.success) throw new AggregateError(result.logs, `build of ${entry} failed`);
  await Bun.write(`${dir}/${out}`, await result.outputs[0]!.text());
}

import { expect, test } from "vitest";
import { readFileSync } from "node:fs";
import { satisfies } from "semver";

type Package = { version: string; peerDependencies?: Record<string, string> };

function installed(name: string): Package {
  return JSON.parse(readFileSync(new URL(`../node_modules/${name}/package.json`, import.meta.url), "utf8"));
}

test("the Solid compiler plugin and web runtime accept the installed runtimes", () => {
  const plugin = installed("@solidjs/vite-plugin");
  const solid = installed("solid-js");
  const web = installed("@solidjs/web");

  for (const [consumer, dependency, version] of [
    [plugin, "solid-js", solid.version],
    [plugin, "@solidjs/web", web.version],
    [web, "solid-js", solid.version],
  ] as const) {
    const range = consumer.peerDependencies?.[dependency];
    expect(range).toBeDefined();
    expect(satisfies(version, range!)).toBe(true);
  }
});

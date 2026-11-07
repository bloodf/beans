import { defineConfig } from "vitest/config";
export default defineConfig({
  plugins: [{
    name: "metro-literal-requires",
    transform(code, id) {
      // Metro bundles these synchronous requires; Vitest must route them through
      // its module executor so each file's native boundary mocks apply.
      const modules = id.endsWith("/src/core/store.ts")
        ? ["../../modules/beans-core"]
        : id.endsWith("/src/i18n/index.ts") ? ["../core/prefs"] : [];
      const imports: string[] = [];
      for (const [index, module] of modules.entries()) {
        const literal = `require(${JSON.stringify(module)})`;
        if (!code.includes(literal)) continue;
        if (id.endsWith("/src/core/store.ts")) {
          code = code.replaceAll(literal, `(await import(${JSON.stringify(module)}))`);
        } else {
          imports.push(`import * as metroRequire${index} from ${JSON.stringify(module)};`);
          code = code.replaceAll(literal, `metroRequire${index}`);
        }
      }
      if (modules.length) return { code: imports.join("\n") + "\n" + code, map: null };
    },
  }],
  oxc: { jsx: { runtime: "automatic", importSource: "react" } },
  test: {
    environment: "node",
    server: { deps: { inline: ["zustand"] } },
    include: ["src/**/*.test.{ts,tsx}"],
    pool: "forks",
    isolate: true,
    maxWorkers: 1,
    fileParallelism: false,
    globals: false,
    clearMocks: false,
    mockReset: false,
    restoreMocks: false,
    passWithNoTests: false,
  },
});

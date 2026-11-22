import { defineConfig } from "vitest/config";
// End-to-end checks of the page in a real Chromium, against its own Vite dev server: the demo
// workspace (`?mock=1`) for every user flow that needs no CLI. `pnpm run test:e2e`.
export default defineConfig({
  test: {
    environment: "node",
    include: ["e2e/**/*.e2e.test.ts"],
    pool: "forks",
    maxWorkers: 1,
    fileParallelism: false,
    testTimeout: 60_000,
    hookTimeout: 120_000,
    passWithNoTests: false,
  },
});

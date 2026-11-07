import { defineConfig } from "vitest/config";
export default defineConfig({
  test: {
    environment: "node",
    include: ["scripts/**/*.test.ts", "web/src/**/*.test.{ts,tsx}", "packages/beans-blobatar/test/**/*.test.ts"],
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

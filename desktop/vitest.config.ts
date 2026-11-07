import { defineConfig } from "vitest/config";
export default defineConfig({
  test: {
    environment: "node",
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

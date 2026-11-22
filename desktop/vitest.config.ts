import solid from "@solidjs/vite-plugin";
import { defineConfig } from "vitest/config";
// Solid compiles the `.tsx` modules a unit test imports (sidebar menus, root selection); `.ts`
// modules are untouched. Mounted consumers still run in Chromium through test-browser.ts.
export default defineConfig({
  plugins: [solid()],
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

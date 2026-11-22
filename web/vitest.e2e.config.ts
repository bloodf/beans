import { defineConfig } from 'vitest/config'
// The site in a real Chromium against its own Vite dev server, after the desktop preview it
// embeds is built (`build:preview`). `pnpm run test:e2e`.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['e2e/**/*.e2e.test.ts'],
    pool: 'forks',
    maxWorkers: 1,
    fileParallelism: false,
    testTimeout: 90_000,
    hookTimeout: 300_000,
    passWithNoTests: false,
    // A first page compiles on demand in the dev server: polls wait for it, not 1 s.
    expect: { poll: { timeout: 20_000, interval: 100 } },
  },
})

import { defineConfig } from 'vitest/config'

// Browser tests: the built SPA (`pnpm build`) in Chrome with a fake backend. Run with `pnpm test:e2e`.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/e2e/**/*.e2e.ts'],
    testTimeout: 60_000,
    hookTimeout: 60_000,
    fileParallelism: false,
  },
})

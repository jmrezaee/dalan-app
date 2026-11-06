import { defineConfig } from 'vitest/config'
import { fileURLToPath } from 'node:url'

const appDir = fileURLToPath(new URL('./app', import.meta.url))

export default defineConfig({
  define: { __DALAN_TEST__: false },
  resolve: { alias: { '@': appDir, '~': appDir } },
  test: { environment: 'happy-dom', include: ['tests/**/*.test.ts'] },
})

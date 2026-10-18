import { fileURLToPath } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'
import baseline from './tests/coverage-baseline.json' with { type: 'json' }

export default defineConfig({
  plugins: [vue()],
  resolve: { alias: { '~': fileURLToPath(new URL('./app', import.meta.url)) } },
  define: { 'import.meta.client': true, __NATIVE__: 'globalThis.__NATIVE__ === true' },
  test: {
    include: ['tests/**/*.test.ts'],
    setupFiles: ['tests/setup.ts'],
    coverage: {
      provider: 'v8',
      include: ['app/composables/**/*.ts', 'app/utils/**/*.ts', 'app/components/**/*.vue'],
      reporter: ['text', 'json-summary', 'lcov'],
      thresholds: baseline.thresholds,
    },
  },
})

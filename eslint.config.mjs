import withNuxt from './.nuxt/eslint.config.mjs'

export default withNuxt(
  {
    ignores: ['src-tauri/**', 'replica/**', 'site/**', 'app/components/ui/**', 'app/types/bindings.ts'],
  },
  {
    // The browser tests poke at `window.__fake` and Tauri's internals, which have no types.
    files: ['tests/e2e/**'],
    rules: { '@typescript-eslint/no-explicit-any': 'off' },
  },
)

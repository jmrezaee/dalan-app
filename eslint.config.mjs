import withNuxt from './.nuxt/eslint.config.mjs'

export default withNuxt(
  {
    ignores: ['src-tauri/**', 'replica/**', 'site/**', 'app/components/ui/**', 'app/types/bindings.ts'],
  },
  {
    // The browser tests poke at `window.__fake` and Tauri's internals, which have no types.
    // Handlers return `undefined` for commands that return nothing, and arguments arrive untyped from the page.
    files: ['tests/e2e/**', 'tests/fake/**', 'tests/walk/**', 'tests/a11y/**', 'tests/visual/**', 'tests/support/**'],
    rules: { '@typescript-eslint/no-explicit-any': 'off', '@typescript-eslint/no-invalid-void-type': 'off' },
  },
)

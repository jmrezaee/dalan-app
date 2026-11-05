import withNuxt from './.nuxt/eslint.config.mjs'

export default withNuxt({
  ignores: ['src-tauri/**', 'replica/**', 'site/**', 'app/components/ui/**', 'app/types/bindings.ts'],
})

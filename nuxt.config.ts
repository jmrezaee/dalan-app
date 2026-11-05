import tailwindcss from '@tailwindcss/vite'

// Tauri serves a static SPA, so no SSR. srcDir is app/ (Nuxt 4 default).
export default defineNuxtConfig({
  compatibilityDate: '2026-11-01',
  ssr: false,
  devtools: { enabled: false },
  modules: ['@pinia/nuxt', '@nuxtjs/color-mode', 'shadcn-nuxt', '@nuxt/eslint'],
  // Folders group components by area, but names stay short (HostCard, not HostsHostCard).
  components: [{ path: '~/components', pathPrefix: false }],
  css: ['~/assets/css/main.css'],
  colorMode: { preference: 'dark', fallback: 'dark', classSuffix: '' },
  shadcn: { prefix: '', componentDir: './app/components/ui' },
  devServer: { port: 4000, host: '127.0.0.1' },
  vite: {
    plugins: [tailwindcss()],
    // Tauri wants a fixed port and readable errors in its own window.
    clearScreen: false,
  },
  ignore: ['**/src-tauri/**'],
  typescript: { strict: true },
})

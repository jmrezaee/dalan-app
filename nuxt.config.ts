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
  app: { head: { title: 'Dalan', htmlAttrs: { lang: 'en' } } },
  colorMode: { preference: 'dark', fallback: 'dark', classSuffix: '' },
  shadcn: { prefix: '', componentDir: './app/components/ui' },
  devServer: { port: 4000, host: '127.0.0.1' },
  // `DALAN_TEST=1 pnpm build` is the browser-test build: it keeps the test hooks (app/utils/testHooks.ts) and writes
  // to .output-test, so a production build in .output never contains them.
  // DALAN_APP_TEST=1 keeps the normal output folder, for the real-app build that Tauri embeds (tests/app).
  nitro: process.env.DALAN_TEST === '1' && process.env.DALAN_APP_TEST !== '1' ? { output: { dir: '.output-test', publicDir: '.output-test/public' } } : {},
  vite: {
    define: { __DALAN_TEST__: JSON.stringify(process.env.DALAN_TEST === '1') },
    plugins: [tailwindcss()],
    // Tauri wants a fixed port and readable errors in its own window.
    clearScreen: false,
  },
  ignore: ['**/src-tauri/**'],
  // /design is the component showcase: dev only, not part of the shipped app.
  hooks: {
    'pages:extend'(pages) {
      if (process.env.NODE_ENV === 'production') {
        const at = pages.findIndex(p => p.path === '/design')
        if (at >= 0) pages.splice(at, 1)
      }
    },
  },
  typescript: { strict: true },
})

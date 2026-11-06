// Replaced at build time (nuxt.config.ts): true only in the browser-test build.
declare const __DALAN_TEST__: boolean

interface Window {
  /** Test builds only: the live xterm instances by session key, so tests can read the buffer. */
  __dalanTerm?: Record<string, unknown>
}

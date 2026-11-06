// Hooks for the browser tests (replica/testing/test-plan.md section 5). Everything here is switched off in a production
// build: `__DALAN_TEST__` is false there, so the bodies are dropped and `window.__dalanTerm` is never written.
// scripts/check-no-test-hooks.mjs fails the build if the name reaches `.output`.

/** `data-testid` value in test builds, nothing otherwise. Use only where a role or label cannot name the element. */
export const tid = (id: string): string | undefined => (__DALAN_TEST__ ? id : undefined)

/** Lets a test read a terminal's buffer (`term.buffer.active`) instead of screenshotting the canvas. */
export function exposeTerm(key: string, term: unknown) {
  if (!__DALAN_TEST__) return
  ;(window.__dalanTerm ??= {})[key] = term
}

export function forgetTerm(key: string) {
  if (!__DALAN_TEST__) return
  if (window.__dalanTerm) Reflect.deleteProperty(window.__dalanTerm, key)
}

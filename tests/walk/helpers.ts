import type { Page } from '@playwright/test'

/** The text in a terminal's buffer, read from the test hook instead of a screenshot of the canvas. */
export const terminalText = (page: Page) => page.evaluate(() => {
  const terms = Object.values((window as any).__dalanTerm ?? {}) as any[]
  // Every terminal on the page (a workspace has several), one after the other.
  return terms.map((term) => {
    const b = term?.buffer.active
    return b ? Array.from({ length: b.length }, (_, i) => b.getLine(i)?.translateToString(true)).join('\n') : ''
  }).join('\n')
})

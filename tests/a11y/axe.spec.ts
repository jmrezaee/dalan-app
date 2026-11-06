import AxeBuilder from '@axe-core/playwright'
import { expect, test } from '../support/fixtures'
import { installFakeBackend } from '../fake'
import { MASTER } from '../fake/commands/vault'
import { ALLOWED } from './allow'
import { SCREENS } from './screens'

// Accessibility (replica/testing/test-plan.md section 9.3): axe on every screen state, zero serious or critical
// violations. What we accept for now is written down, with a reason, in allow.ts.
// Dark is the default; the screens people spend their time in are checked in light too (contrast differs).
const LIGHT = new Set(['S24-unlock', 'S02-grid', 'S03-new-host', 'S05-keychain', 'S18-connected', 'S21-sftp', 'S23-settings-terminal'])
const runs = SCREENS.flatMap(s => [{ s, theme: 'dark' as const }, ...(LIGHT.has(s.id) ? [{ s, theme: 'light' as const }] : [])])

for (const { s, theme } of runs) {
  test(`axe: ${s.id}${theme === 'light' ? ' (light)' : ''}`, async ({ page }) => {
    const fake = await installFakeBackend(page, { scenario: s.scenario, unlocked: s.unlocked ?? true, theme })
    await page.goto('/')
    if (s.unlocked === false) await expect(page.getByLabel('Master password')).toBeVisible()
    else await expect(page.getByLabel('Vault', { exact: true })).toBeVisible()
    await s.open(page, fake)
    await page.waitForTimeout(400) // transitions
    const result = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze()
    const bad = result.violations
      .filter(v => v.impact === 'serious' || v.impact === 'critical')
      .map(v => ({ ...v, nodes: v.nodes.filter(n => !ALLOWED.some(a => a.rule === v.id && (!a.state || a.state === s.id) && (!a.selector || n.target.join(' ').includes(a.selector)))) }))
      .filter(v => v.nodes.length)
    expect(bad.map(v => `${v.id} (${v.impact}): ${v.help} @ ${v.nodes.map(n => n.target.join(' ')).slice(0, 3).join(' | ')}`)).toEqual([])
  })
}
void MASTER

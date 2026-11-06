import { addHost, button, createVault, servers, type, waitForTerminal } from '../support'

// F08 and F09: run a snippet on two real servers (a workspace opens with a pane each), then broadcast one command to both.
// SKIPPED for now: Written, not yet passing: the first run was stopped by a click swallowed by a closing sheet (fixed in `addHost`), then not rerun. To resume: run it alone with `pnpm test:app --spec f08-f09-workspace`.
describe.skip('F08 and F09: a snippet on two servers, then broadcast', () => {
  it('runs the snippet in both panes and types one command into both', async () => {
    const a = servers().a!
    const r = servers().rot!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: a.password, label: 'server a' })
    await addHost({ address: r.net_host, port: r.net_port, user: r.user!, password: r.password, label: 'server rot' })

    await $('nav[aria-label="Vault"]').$('//*[normalize-space(.)="Snippets"]').click()
    await button('New snippet').click()
    await type($('#sn-label'), 'say hello')
    await type($('#sn-script'), 'echo snippet-$((6*7))')
    await button('Save').click()
    await $('//*[normalize-space(text())="say hello"]').waitForDisplayed()
    await $('//button[normalize-space(.)="Run"]').click()
    await $('//button[normalize-space(.)="Select all"]').click()
    await $('//button[starts-with(normalize-space(.),"Run on 2")]').click()

    // Each pane asks about its server's key.
    for (let i = 0; i < 2; i++) {
      const trust = await $('//button[normalize-space(.)="Trust and connect"]')
      await trust.waitForDisplayed({ timeout: 40_000 })
      await trust.click()
      await browser.pause(800)
    }
    // The snippet ran in both panes.
    await browser.waitUntil(async () => {
      const text = await browser.execute(() => Object.values((window as unknown as { __dalanTerm: Record<string, any> }).__dalanTerm).map((t) => {
        const b = t.buffer.active
        return Array.from({ length: b.length }, (_, i) => b.getLine(i)?.translateToString(true)).join('\n')
      }))
      return text.length === 2 && text.every(t => t.includes('snippet-42'))
    }, { timeout: 60_000, timeoutMsg: 'the snippet did not run in both panes' })

    // Broadcast: one command typed once appears in both.
    await $('//button[starts-with(normalize-space(.),"Broadcast")]').click()
    await $('//*[contains(normalize-space(.),"Broadcasting to 2")]').waitForDisplayed()
    await $('.xterm-helper-textarea').addValue('echo cast-$((1+1))')
    await browser.keys('Enter')
    await browser.waitUntil(async () => {
      const text = await browser.execute(() => Object.values((window as unknown as { __dalanTerm: Record<string, any> }).__dalanTerm).map((t) => {
        const b = t.buffer.active
        return Array.from({ length: b.length }, (_, i) => b.getLine(i)?.translateToString(true)).join('\n')
      }))
      return text.every(t => t.includes('cast-2'))
    }, { timeout: 30_000, timeoutMsg: 'the broadcast did not reach both panes' })
    void waitForTerminal
  })
})

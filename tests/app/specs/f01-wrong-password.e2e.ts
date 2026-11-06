import { addHost, button, byText, createVault, heading, servers } from '../support'

// F01: a wrong saved password is a normal sign-in failure, with a way to try again, against a real sshd.
describe('F01: signing in with a wrong password', () => {
  it('says the sign-in failed and offers another try', async () => {
    const a = servers().a!
    await createVault()
    await addHost({ address: a.net_host, port: a.net_port, user: a.user!, password: 'not-the-password', label: 'wrong pw' })
    await (await byText('wrong pw')).doubleClick()
    await heading('First time connecting to wrong pw').waitForDisplayed()
    await button('Trust and connect').click()
    await heading('Sign-in failed').waitForDisplayed({ timeout: 30_000 })
    await button('Try again').waitForDisplayed()
  })
})

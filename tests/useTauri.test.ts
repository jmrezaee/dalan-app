import { describe, expect, it } from 'vitest'
import { CommandError, unwrap } from '../app/composables/useTauri'

describe('unwrap', () => {
  it('returns data on ok', async () => {
    expect(await unwrap(Promise.resolve({ status: 'ok' as const, data: 7 }))).toBe(7)
  })
  it('throws CommandError with the code on error', async () => {
    const err = { status: 'error' as const, error: { code: 'locked' as const, message: 'The vault is locked.' } }
    await expect(unwrap(Promise.resolve(err))).rejects.toMatchObject({ code: 'locked' })
    await expect(unwrap(Promise.resolve(err))).rejects.toBeInstanceOf(CommandError)
  })
})

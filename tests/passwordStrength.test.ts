import { describe, expect, it } from 'vitest'
import { passwordStrength } from '../app/utils/passwordStrength'

describe('passwordStrength', () => {
  it('scores empty, short, ok and strong passwords', () => {
    expect(passwordStrength('')).toBe(0)
    expect(passwordStrength('abc')).toBe(1)
    expect(passwordStrength('abcdefgh')).toBe(1)
    expect(passwordStrength('correct-horse-staple')).toBe(5)
    expect(passwordStrength('Tr0ub4dor&3')).toBe(3)
  })
})

import { describe, expect, it } from 'vitest'
import { actionFor, comboProblem, effectiveCombos, eventCombo, formatCombo } from '~/utils/shortcuts'

const key = (key: string, m: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey', boolean>> = {}) => ({ key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...m })

describe('shortcuts', () => {
  it('turns key presses into combos, per platform', () => {
    expect(eventCombo(key('m', { metaKey: true, shiftKey: true }), true)).toBe('Mod+Shift+M')
    expect(eventCombo(key('k', { ctrlKey: true }), false)).toBe('Mod+K')
    expect(eventCombo(key('k', { ctrlKey: true }), true)).toBe('Ctrl+K') // Ctrl on a Mac is not Mod
    expect(eventCombo(key(',', { metaKey: true }), true)).toBe('Mod+,')
    expect(eventCombo(key('Meta', { metaKey: true }), true)).toBeNull() // only a modifier so far
  })
  it('formats combos', () => {
    expect(formatCombo('Mod+Shift+M', true)).toBe('⌘⇧M')
    expect(formatCombo('Mod+Shift+M', false)).toBe('Ctrl+Shift+M')
    expect(formatCombo('Mod+,', true)).toBe('⌘,')
  })
  it('applies overrides and finds the action', () => {
    const combos = effectiveCombos({ 'new-tab': 'Mod+Y' })
    expect(actionFor(key('y', { metaKey: true }), combos, true)).toBe('new-tab')
    expect(actionFor(key('t', { metaKey: true }), combos, true)).toBeNull() // the old default is free now
    expect(actionFor(key('k', { metaKey: true }), combos, true)).toBe('palette')
  })
  it('refuses reserved, tab-number, modifier-less and taken combos', () => {
    const combos = effectiveCombos({})
    expect(comboProblem('palette', 'Mod+C', combos)).toMatch(/copy/)
    expect(comboProblem('palette', 'Mod+3', combos)).toMatch(/switch tabs/)
    expect(comboProblem('palette', 'K', combos)).toMatch(/key/)
    expect(comboProblem('palette', 'Mod+T', combos)).toMatch(/New tab/)
    expect(comboProblem('palette', 'Mod+K', combos)).toBeNull() // its own combo is fine
    expect(comboProblem('palette', 'Mod+Y', combos)).toBeNull()
  })
})

import { describe, expect, it } from 'vitest'
import { cn } from '@/lib/utils'
import { osLetter, osName } from '~/utils/os'

describe('cn', () => {
  it('merges conflicting tailwind classes', () => {
    expect(cn('p-2', 'p-4')).toBe('p-4')
  })
})

describe('os icon', () => {
  it('maps detected names to a letter and a title', () => {
    expect(osLetter('ubuntu')).toBe('U')
    expect(osLetter('macos')).toBe('M')
    expect(osName('rhel')).toBe('Red Hat')
    expect(osName('gentoo')).toBe('Gentoo') // unlisted names still read well
    expect(osLetter('unknown')).toBeUndefined()
    expect(osLetter(null)).toBeUndefined()
  })
})

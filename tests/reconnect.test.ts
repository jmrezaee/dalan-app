import { describe, expect, it } from 'vitest'
import { retryDelayMs } from '../app/utils/reconnect'

describe('automatic reconnect schedule', () => {
  it('waits longer each time and then gives up', () => {
    expect([0, 1, 2, 3, 4].map(retryDelayMs)).toEqual([2000, 4000, 8000, 16000, 30000])
    expect(retryDelayMs(5)).toBeNull()
  })
})

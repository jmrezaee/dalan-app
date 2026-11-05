import { describe, expect, it } from 'vitest'
import { parseQuickConnect } from '../app/utils/quickConnect'

describe('parseQuickConnect', () => {
  it('reads user, host and port', () => {
    expect(parseQuickConnect('me@10.0.0.5:2222')).toEqual({ username: 'me', address: '10.0.0.5', port: 2222 })
    expect(parseQuickConnect('me@box')).toEqual({ username: 'me', address: 'box', port: null })
    expect(parseQuickConnect('example.com')).toEqual({ username: null, address: 'example.com', port: null })
  })
  it('leaves plain words and bad ports to the search', () => {
    expect(parseQuickConnect('web')).toBeNull()
    expect(parseQuickConnect('me@box:99999')).toBeNull()
    expect(parseQuickConnect('two words')).toBeNull()
  })
})

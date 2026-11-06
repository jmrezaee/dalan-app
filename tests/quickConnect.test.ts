import { describe, expect, it } from 'vitest'
import { findQuickHost, parseQuickConnect } from '../app/utils/quickConnect'

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

describe('findQuickHost', () => {
  const h = (address: string, user: string | null, port: number) => ({ address, effective_username: user, effective_port: port })
  const list = [h('box', 'me', 2222), h('box', 'me', 22), h('other', 'me', 22)]
  it('matches address, user and port, and treats no port as 22', () => {
    expect(findQuickHost(list, { username: 'me', address: 'box', port: 2222 })).toBe(list[0])
    expect(findQuickHost(list, { username: 'me', address: 'BOX', port: null })).toBe(list[1])
  })
  it('finds nothing for another user or port, so a new host is saved', () => {
    expect(findQuickHost(list, { username: 'you', address: 'box', port: null })).toBeUndefined()
    expect(findQuickHost(list, { username: 'me', address: 'box', port: 2200 })).toBeUndefined()
  })
})

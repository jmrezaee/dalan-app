import { describe, expect, it } from 'vitest'
import { paneDot, paneDotLabel, workspaceDot } from '../app/utils/sessionDot'

const pane = (over: Partial<Parameters<typeof paneDot>[0]> = {}) => ({ status: 'ready' as const, prompt: null, unread: false, ...over })
const prompt = { id: 'p', kind: { type: 'password' as const, host: 'h', user: 'u' } }

describe('tab dots', () => {
  it('is quiet when connected and nothing new arrived', () => {
    expect(paneDot(pane())).toBe('idle')
    expect(paneDot(pane({ unread: true }))).toBe('unread')
  })
  it('waits for you when a question is open, and is red when failed or disconnected', () => {
    expect(paneDot(pane({ prompt, status: 'authenticating' }))).toBe('waiting')
    expect(paneDot(pane({ status: 'error' }))).toBe('error')
    expect(paneDot(pane({ status: 'disconnected' }))).toBe('error')
    expect(paneDot(pane({ status: 'connecting' }))).toBe('idle')
  })
  it('labels say what the colour means', () => {
    expect(paneDotLabel(pane({ prompt }), 'web-1')).toBe('web-1: waiting for input')
    expect(paneDotLabel(pane({ status: 'disconnected' }))).toBe('disconnected')
  })
  it('a workspace shows its most urgent pane', () => {
    expect(workspaceDot([pane(), pane({ unread: true })])).toBe('unread')
    expect(workspaceDot([pane({ unread: true }), pane({ status: 'error' })])).toBe('error')
    expect(workspaceDot([pane({ status: 'error' }), pane({ prompt })])).toBe('waiting')
    expect(workspaceDot([])).toBe('idle')
  })
})

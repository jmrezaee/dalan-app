import { describe, expect, it } from 'vitest'
import { repeatedCommand } from '../app/utils/historyHint'

describe('repeatedCommand', () => {
  const cmds = ['docker ps', 'ls', 'ls', 'ls', 'docker ps', 'docker ps', 'docker ps', 'tail -f log', 'tail -f log', 'tail -f log']
  it('picks the most repeated command and ignores very short ones', () => {
    expect(repeatedCommand(cmds, [], new Set())).toEqual({ command: 'docker ps', count: 4 })
  })
  it('skips saved and dismissed commands, and needs three repeats', () => {
    expect(repeatedCommand(cmds, ['docker ps'], new Set())).toEqual({ command: 'tail -f log', count: 3 })
    expect(repeatedCommand(cmds, ['docker ps'], new Set(['tail -f log']))).toBeNull()
    expect(repeatedCommand(['a long one', 'a long one'], [], new Set())).toBeNull()
  })
})

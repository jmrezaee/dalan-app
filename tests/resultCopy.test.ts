import { describe, expect, it } from 'vitest'
import { copyAs } from '../app/utils/resultCopy'
import type { SqlCell_Serialize as Cell } from '../app/types/bindings'

const c = (kind: Cell['kind'], v: string | null): Cell => ({ kind, v, cut: false })
const names = ['id', 'name', 'note', 'ok']
const rows: Cell[][] = [
  [c('int', '1'), c('text', 'Sara, "S" Moradi'), c('null', null), c('bool', 'true')],
  [c('int', '9223372036854775807'), c('text', 'it\'s'), c('json', '{"a":1}'), c('bool', 'false')],
]

describe('copy as', () => {
  it('CSV quotes what needs it and leaves NULL empty', () => {
    expect(copyAs('csv', names, rows)).toBe('id,name,note,ok\r\n1,"Sara, ""S"" Moradi",,true\r\n9223372036854775807,it\'s,"{""a"":1}",false')
  })
  it('TSV has no tabs or newlines inside a value', () => {
    expect(copyAs('tsv', ['a'], [[c('text', 'x\ty\nz')]])).toBe('a\nx y z')
  })
  it('JSON keeps a bigint as text, parses json and turns NULL into null', () => {
    const out = JSON.parse(copyAs('json', names, rows))
    expect(out[0]).toEqual({ id: 1, name: 'Sara, "S" Moradi', note: null, ok: true })
    expect(out[1]).toMatchObject({ id: '9223372036854775807', note: { a: 1 }, ok: false })
  })
  it('Markdown escapes the bar', () => {
    expect(copyAs('markdown', ['a'], [[c('text', 'x|y')]])).toBe('| a |\n| --- |\n| x\\|y |')
  })
  it('SQL insert quotes text, doubles quotes and writes NULL', () => {
    expect(copyAs('sql', names, rows, 'people').split('\n')[0]).toBe('insert into "people" ("id", "name", "note", "ok") values (1, \'Sara, "S" Moradi\', NULL, true);')
    expect(copyAs('sql', ['n'], [[c('text', 'it\'s')]])).toContain('\'it\'\'s\'')
  })
})

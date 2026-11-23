// "Copy as…" for a result set (S37): CSV, TSV, JSON, Markdown and SQL insert. Pure text, so it is tested without a window.
// Values are the server's text forms; NULL stays NULL (empty in CSV/TSV, null in JSON, NULL in SQL).
import type { SqlCell_Serialize as Cell } from '~/types/bindings'

export type CopyFormat = 'csv' | 'tsv' | 'json' | 'markdown' | 'sql'
export const COPY_FORMATS: { id: CopyFormat, label: string }[] = [
  { id: 'csv', label: 'CSV' },
  { id: 'tsv', label: 'TSV' },
  { id: 'json', label: 'JSON' },
  { id: 'markdown', label: 'Markdown' },
  { id: 'sql', label: 'SQL insert' },
]

const NUMBER_KINDS = new Set(['int', 'decimal', 'float'])

function csvField(v: string | null) {
  if (v === null) return ''
  return /[",\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v
}

/** A cell as a JSON value: numbers and booleans as themselves (a bigint stays a string so no digit is lost), json parsed. */
function jsonValue(c: Cell): unknown {
  if (c.v === null) return null
  if (c.kind === 'bool') return c.v === 'true' || c.v === 't'
  if (c.kind === 'int' && Number.isSafeInteger(Number(c.v))) return Number(c.v)
  if (c.kind === 'float' && Number.isFinite(Number(c.v))) return Number(c.v)
  if (c.kind === 'json') {
    try { return JSON.parse(c.v) }
    catch { return c.v }
  }
  return c.v
}

function sqlLiteral(c: Cell) {
  if (c.v === null) return 'NULL'
  if (NUMBER_KINDS.has(c.kind) && /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(c.v)) return c.v
  if (c.kind === 'bool' && (c.v === 'true' || c.v === 'false')) return c.v
  return `'${c.v.replace(/'/g, '\'\'')}'`
}
const quoteName = (n: string) => `"${n.replace(/"/g, '""')}"`

export function copyAs(format: CopyFormat, names: string[], rows: Cell[][], table = 'table_name'): string {
  switch (format) {
    case 'csv':
      return [names.map(csvField), ...rows.map(r => r.map(c => csvField(c.v)))].map(r => r.join(',')).join('\r\n')
    case 'tsv': {
      const t = (v: string | null) => (v ?? '').replace(/[\t\r\n]+/g, ' ')
      return [names.map(t), ...rows.map(r => r.map(c => t(c.v)))].map(r => r.join('\t')).join('\n')
    }
    case 'json':
      return JSON.stringify(rows.map(r => Object.fromEntries(names.map((n, i) => [n, r[i] ? jsonValue(r[i]) : null]))), null, 2)
    case 'markdown': {
      const m = (v: string | null) => (v ?? '').replace(/\|/g, '\\|').replace(/[\r\n]+/g, ' ')
      return [`| ${names.map(m).join(' | ')} |`, `| ${names.map(() => '---').join(' | ')} |`, ...rows.map(r => `| ${r.map(c => m(c.v)).join(' | ')} |`)].join('\n')
    }
    case 'sql':
      return rows.map(r => `insert into ${quoteName(table)} (${names.map(quoteName).join(', ')}) values (${r.map(sqlLiteral).join(', ')});`).join('\n')
  }
}

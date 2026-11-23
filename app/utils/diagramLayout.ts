// The diagram (S40): where each table box goes and how the keys connect them, and the same picture as a standalone
// SVG file. Pure functions, so layout and export are tested without a window. Boxes are placed in columns by their
// distance from the first table; edges run from the foreign key's row to the key it points at.
import type { Diagram } from '~/types/bindings'

export const BOX_W = 232
export const HEAD = 30
export const ROW = 20
const GAP_X = 96
const GAP_Y = 28
const PAD = 24

export interface Box { id: string, schema: string, name: string, x: number, y: number, w: number, h: number, rows: { name: string, type_name: string, pk: boolean, fk: boolean }[] }
export interface Link { name: string, d: string, label: string, from: string, to: string }
export interface Layout { boxes: Box[], links: Link[], width: number, height: number }

const idOf = (schema: string, name: string) => `${schema}.${name}`

export function layout(d: Diagram, start: string[], keysOnly: boolean): Layout {
  const ids = d.tables.map(t => idOf(t.schema, t.name))
  // Columns by hops from the start tables.
  const depth = new Map<string, number>()
  const roots = start.filter(s => ids.includes(s))
  for (const r of roots.length ? roots : ids.slice(0, 1)) depth.set(r, 0)
  for (let level = 0; depth.size < ids.length && level < ids.length; level++) {
    let grew = false
    for (const e of d.edges) {
      const a = idOf(e.from_schema, e.from_table), b = idOf(e.to_schema, e.to_table)
      for (const [x, y] of [[a, b], [b, a]] as const) {
        if (depth.get(x) === level && !depth.has(y) && ids.includes(y)) { depth.set(y, level + 1); grew = true }
      }
    }
    if (!grew) break
  }
  for (const id of ids) if (!depth.has(id)) depth.set(id, Math.max(0, ...depth.values()) + 1)
  const colHeights: number[] = []
  const boxes: Box[] = d.tables.map((t) => {
    const id = idOf(t.schema, t.name)
    const rows = t.columns.filter(c => !keysOnly || c.pk || c.fk).map(c => ({ name: c.name, type_name: c.type_name, pk: c.pk, fk: c.fk }))
    const col = depth.get(id) ?? 0
    const h = HEAD + Math.max(1, rows.length) * ROW + 6
    const y = colHeights[col] ?? PAD
    colHeights[col] = y + h + GAP_Y
    return { id, schema: t.schema, name: t.name, x: PAD + col * (BOX_W + GAP_X), y, w: BOX_W, h, rows }
  })
  const byId = new Map(boxes.map(b => [b.id, b]))
  const rowY = (b: Box, col: string | undefined) => {
    const i = Math.max(0, b.rows.findIndex(r => r.name === col))
    return b.y + HEAD + i * ROW + ROW / 2
  }
  const links: Link[] = []
  for (const e of d.edges) {
    const a = byId.get(idOf(e.from_schema, e.from_table)), b = byId.get(idOf(e.to_schema, e.to_table))
    if (!a || !b) continue
    const ay = rowY(a, e.from_columns[0]), by = rowY(b, e.to_columns[0])
    // Leave on the side that faces the other box; boxes in one column loop out to the right.
    let path: string
    if (a.x === b.x) {
      const x = a.x + a.w
      path = `M ${x} ${ay} C ${x + 48} ${ay}, ${x + 48} ${by}, ${x} ${by}`
    }
    else {
      const [ax, bx] = a.x < b.x ? [a.x + a.w, b.x] : [a.x, b.x + b.w]
      const k = (bx - ax) / 2
      path = `M ${ax} ${ay} C ${ax + k} ${ay}, ${bx - k} ${by}, ${bx} ${by}`
    }
    links.push({ name: e.name, d: path, label: `${e.from_table}.${e.from_columns.join(',')} → ${e.to_table}.${e.to_columns.join(',')}`, from: a.id, to: b.id })
  }
  const width = Math.max(320, ...boxes.map(b => b.x + b.w + PAD + 48))
  const height = Math.max(200, ...boxes.map(b => b.y + b.h + PAD))
  return { boxes, links, width, height }
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

/** A standalone SVG of the layout. `c` carries the colours (read from the theme at export time, never written here). */
export function toSvg(l: Layout, c: { bg: string, surface: string, border: string, text: string, muted: string, accent: string }): string {
  const parts = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${l.width}" height="${l.height}" viewBox="0 0 ${l.width} ${l.height}" font-family="Inter, sans-serif" font-size="12">`,
    `<rect width="100%" height="100%" fill="${c.bg}"/>`,
  ]
  for (const k of l.links) parts.push(`<path d="${k.d}" fill="none" stroke="${c.accent}" stroke-width="1.5"><title>${esc(k.label)}</title></path>`)
  for (const b of l.boxes) {
    parts.push(`<g><rect x="${b.x}" y="${b.y}" width="${b.w}" height="${b.h}" rx="8" fill="${c.surface}" stroke="${c.border}"/>`)
    parts.push(`<text x="${b.x + 12}" y="${b.y + 20}" font-weight="600" fill="${c.text}">${esc(b.schema === 'public' ? b.name : `${b.schema}.${b.name}`)}</text>`)
    b.rows.forEach((r, i) => {
      const y = b.y + HEAD + i * ROW + 14
      parts.push(`<text x="${b.x + 12}" y="${y}" font-family="monospace" fill="${r.pk || r.fk ? c.accent : c.text}">${esc(r.name)}</text><text x="${b.x + b.w - 12}" y="${y}" text-anchor="end" font-family="monospace" fill="${c.muted}">${esc(r.type_name)}</text>`)
    })
    parts.push('</g>')
  }
  parts.push('</svg>')
  return parts.join('\n')
}

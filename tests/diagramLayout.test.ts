import { describe, expect, it } from 'vitest'
import { layout, toSvg, BOX_W } from '../app/utils/diagramLayout'
import type { Diagram } from '../app/types/bindings'

const col = (name: string, pk = false, fk = false) => ({ name, type_name: 'bigint', pk, fk })
const d: Diagram = {
  total_tables: 9,
  tables: [
    { schema: 'public', name: 'orders', columns: [col('id', true), col('customer_id', false, true), col('total')] },
    { schema: 'public', name: 'customers', columns: [col('id', true), col('name')] },
    { schema: 'billing', name: 'refunds<x>', columns: [col('id', true), col('order_id', false, true)] },
  ],
  edges: [
    { name: 'o_c', from_schema: 'public', from_table: 'orders', from_columns: ['customer_id'], to_schema: 'public', to_table: 'customers', to_columns: ['id'], on_delete: 'restrict', on_update: 'no action', indexed: true },
    { name: 'r_o', from_schema: 'billing', from_table: 'refunds<x>', from_columns: ['order_id'], to_schema: 'public', to_table: 'orders', to_columns: ['id'], on_delete: 'cascade', on_update: 'no action', indexed: false },
  ],
}

describe('diagram layout', () => {
  it('puts the start table first and its neighbours in the next column', () => {
    const l = layout(d, ['public.orders'], false)
    const x = (id: string) => l.boxes.find(b => b.id === id)!.x
    expect(x('public.orders')).toBeLessThan(x('public.customers'))
    expect(x('public.customers')).toBe(x('billing.refunds<x>'))
    expect(l.links).toHaveLength(2)
    expect(l.width).toBeGreaterThan(2 * BOX_W)
  })
  it('keys only keeps the key columns', () => {
    const l = layout(d, ['public.orders'], true)
    expect(l.boxes.find(b => b.name === 'orders')!.rows.map(r => r.name)).toEqual(['id', 'customer_id'])
  })
  it('the SVG is standalone, takes its colours from the caller and escapes names', () => {
    const svg = toSvg(layout(d, ['public.orders'], false), { bg: 'BG', surface: 'SF', border: 'BD', text: 'TX', muted: 'MU', accent: 'AC' })
    expect(svg.startsWith('<svg xmlns')).toBe(true)
    expect(svg).toContain('fill="BG"')
    expect(svg).toContain('billing.refunds&lt;x&gt;')
    expect(svg).not.toContain('refunds<x>')
    expect(svg).toContain('<title>orders.customer_id → customers.id</title>')
  })
})

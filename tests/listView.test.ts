import { describe, expect, it } from 'vitest'
import { filterSort } from '../app/utils/listView'

describe('filterSort', () => {
  const items = ['web-10', 'Web-2', 'db']
  it('filters without case and sorts by name with numbers in order', () => {
    expect(filterSort(items, '', false, s => s)).toEqual(['db', 'Web-2', 'web-10'])
    expect(filterSort(items, 'WEB', true, s => s)).toEqual(['web-10', 'Web-2'])
  })
})

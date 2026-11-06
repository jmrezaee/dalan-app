/** Filter by text and sort by name, for the section lists (Keychain, Snippets, Port forwarding, Known hosts). */
export function filterSort<T>(items: T[], query: string, desc: boolean, name: (t: T) => string, text: (t: T) => string = name): T[] {
  const q = query.trim().toLowerCase()
  const out = q ? items.filter(i => text(i).toLowerCase().includes(q)) : [...items]
  return out.sort((a, b) => name(a).localeCompare(name(b), undefined, { sensitivity: 'base', numeric: true }) * (desc ? -1 : 1))
}

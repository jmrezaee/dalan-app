/** Reads `user@host`, `host:port` or `user@host:port`. Returns null for anything that is not a plain address. */
export function parseQuickConnect(text: string): { username: string | null, address: string, port: number | null } | null {
  const m = /^(?:([\w.-]+)@)?([\w.-]+|\[[0-9a-f:]+\])(?::(\d{1,5}))?$/i.exec(text.trim())
  if (!m) return null
  const port = m[3] ? Number(m[3]) : null
  if (port !== null && (port < 1 || port > 65535)) return null
  // A bare word with no dot, port or user is more likely a search than an address.
  if (!m[1] && !m[3] && !/[.:]/.test(m[2]!)) return null
  return { username: m[1] ?? null, address: m[2]!.replace(/^\[|\]$/g, ''), port }
}

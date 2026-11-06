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

/** The saved host a quick-connect text points at: same address, user and port (no port = 22). Newest first wins. */
export function findQuickHost<H extends { address: string, effective_username?: string | null, effective_port: number }>(
  hosts: H[],
  q: { username: string | null, address: string, port: number | null },
): H | undefined {
  return hosts.find(h =>
    h.address.toLowerCase() === q.address.toLowerCase()
    && (h.effective_username ?? null) === q.username
    && h.effective_port === (q.port ?? 22),
  )
}

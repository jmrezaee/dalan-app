/** The most repeated command (3 times or more) that is not already a snippet and was not dismissed. */
export function repeatedCommand(commands: string[], snippetScripts: string[], dismissed: Set<string>, min = 3): { command: string, count: number } | null {
  const counts = new Map<string, number>()
  for (const raw of commands) {
    const c = raw.trim()
    if (c.length < 4) continue // "ls", "cd" are not worth saving
    counts.set(c, (counts.get(c) ?? 0) + 1)
  }
  let best: { command: string, count: number } | null = null
  for (const [command, count] of counts) {
    if (count < min || dismissed.has(command) || snippetScripts.includes(command)) continue
    if (!best || count > best.count) best = { command, count }
  }
  return best
}

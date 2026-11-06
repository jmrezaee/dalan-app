// The story, in order. `flows` are the flow and screen IDs a chapter covers (`pnpm walk --from F07`, `--only F11,F12`).
export interface ChapterInfo { n: number, title: string, flows: string[] }

export const CHAPTERS: ChapterInfo[] = [
  { n: 1, title: 'First run', flows: ['F10', 'S24'] },
  { n: 2, title: 'Import', flows: ['F10', 'S25'] },
  { n: 3, title: 'Organise', flows: ['F04', 'S02', 'S03', 'S04'] },
  { n: 4, title: 'Keychain', flows: ['F02', 'S05', 'S06', 'S07', 'S08'] },
  { n: 5, title: 'Connect', flows: ['F01', 'S18', 'S20'] },
  { n: 6, title: 'Sign-in problems', flows: ['F01', 'S18'] },
  { n: 7, title: 'Navigation', flows: ['F03', 'S01', 'S16', 'S17'] },
  { n: 8, title: 'Jump host', flows: ['F05'] },
  { n: 9, title: 'Workspace', flows: ['F08', 'F09', 'S13', 'S19'] },
  { n: 10, title: 'Drops', flows: ['F11'] },
  { n: 11, title: 'Identity changed', flows: ['F12', 'S14'] },
  { n: 12, title: 'Tunnels', flows: ['F06', 'S09', 'S10'] },
  { n: 13, title: 'Files', flows: ['F07', 'S21', 'S22'] },
  { n: 14, title: 'Logs and history', flows: ['S15', 'S20'] },
  { n: 15, title: 'Settings', flows: ['S23'] },
  { n: 16, title: 'Light theme', flows: ['S02', 'S03', 'S18', 'S19', 'S21'] },
  { n: 17, title: 'Lock', flows: ['S24'] },
]

/** Which chapters to run, from the WALK_FROM and WALK_ONLY the run script sets. */
export function selected(n: number, env: Record<string, string | undefined> = process.env): boolean {
  const me = CHAPTERS.find(c => c.n === n)!
  const only = env.WALK_ONLY?.split(',').map(s => s.trim()).filter(Boolean)
  if (only?.length && !only.some(o => o === String(n) || me.flows.includes(o))) return false
  const from = env.WALK_FROM
  if (from) {
    const start = CHAPTERS.find(c => String(c.n) === from || c.flows.includes(from))
    if (start && n < start.n) return false
  }
  return true
}

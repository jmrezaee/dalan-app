import type { Session } from '~/stores/sessions'

export type Dot = 'unread' | 'waiting' | 'error' | 'idle'

/** The tab dot for one connection: waiting for you > failed > new output > quiet (components.md, StatusDot). */
export function paneDot(p: Pick<Session, 'status' | 'prompt' | 'unread'>): Dot {
  if (p.prompt) return 'waiting'
  if (p.status === 'error' || p.status === 'disconnected') return 'error'
  if (p.status === 'ready' && p.unread) return 'unread'
  return 'idle'
}

export function paneDotLabel(p: Pick<Session, 'status' | 'prompt' | 'unread'>, name = ''): string {
  const what = p.prompt ? 'waiting for input'
    : p.status === 'error' ? 'connection failed'
      : p.status === 'disconnected' ? 'disconnected'
        : p.status === 'connecting' ? 'connecting'
          : p.status === 'authenticating' ? 'signing in'
            : p.unread ? 'new output' : 'connected'
  return name ? `${name}: ${what}` : what
}

const RANK: Dot[] = ['waiting', 'error', 'unread', 'idle']

/** A workspace tab shows the most urgent dot among its panes. */
export function workspaceDot(panes: Pick<Session, 'status' | 'prompt' | 'unread'>[]): Dot {
  const dots = panes.map(paneDot)
  return RANK.find(d => dots.includes(d)) ?? 'idle'
}

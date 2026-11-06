// Terminal sessions. The fake never connects by itself: a test (or a scripted server) sends the events a real
// connection would, with `fake.session(id)`. The commands only record what the UI asked for.
import { fail, type Handlers } from '../types'

type Keys = 'sessionOpen' | 'localSessionOpen' | 'sessionWrite' | 'sessionResize' | 'sessionAck' | 'sessionRespond' | 'sessionClose'
  | 'sessionReconnect' | 'sessionReconnectWith' | 'keyExportToHost'

export const sessions: Pick<Handlers, Keys> = {
  sessionOpen: (a, { m, id, channel }) => {
    if (!m.hosts.some(h => h.id === a.hostId)) fail('not_found', 'That host is gone.')
    const sid = id('fake')
    m.sessions.set(sid, { events: channel(a.events), data: channel(a.data), host: a.hostId, closed: false })
    return { session_id: sid }
  },
  localSessionOpen: (a, { m, id, channel }) => {
    const sid = id('fake')
    m.sessions.set(sid, { events: channel(a.events), data: channel(a.data), host: '', closed: false })
    return { session_id: sid }
  },
  sessionWrite: (a, { m }) => {
    m.written.push({ id: a.sessionId, text: new TextDecoder().decode(new Uint8Array(a.data ?? [])) })
    return null
  },
  sessionResize: () => null,
  sessionAck: (a, { tag }) => {
    tag(`ack:${a.bytes}`)
    return null
  },
  sessionRespond: (a, { m }) => {
    m.responses.push({ id: a.sessionId, prompt: a.promptId, answer: a.answer })
    return null
  },
  sessionClose: (a, { m }) => {
    const s = m.sessions.get(a.sessionId)
    if (s) s.closed = true
  },
  sessionReconnect: (a, { tag }) => {
    tag(`reconnect:${a.sessionId}`)
    return null
  },
  sessionReconnectWith: (a, { tag }) => {
    tag(`reconnect:${a.sessionId}:${a.method}`)
    return null
  },
  keyExportToHost: (a, { m, id, channel }) => {
    if (!m.keys.some(k => k.id === a.keyId)) fail('not_found', 'That key is gone.')
    if (!m.hosts.some(h => h.id === a.hostId)) fail('not_found', 'That host is gone.')
    const sid = id('fake')
    m.sessions.set(sid, { events: channel(a.events), data: null, host: a.hostId, closed: false })
    return { session_id: sid }
  },
}

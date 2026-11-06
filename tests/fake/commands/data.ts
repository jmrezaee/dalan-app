// Snippets, port forwarding, workspaces and logs: plain lists with a few rules.
import type { ForwardSummary_Serialize } from '../../../app/types/bindings'
import type { Model } from '../model'
import { fail, type Handlers } from '../types'

type Keys = 'snippetsList' | 'snippetSave' | 'snippetDelete' | 'snippetsMove' | 'packageSave' | 'packageDelete'
  | 'forwardsList' | 'forwardSave' | 'forwardDelete' | 'forwardStart' | 'forwardStop' | 'forwardRespond'
  | 'workspacesList' | 'workspaceSave' | 'workspaceDelete' | 'logsList' | 'logBookmark' | 'logDelete' | 'logReplay'

const forwardSummary = (m: Model, f: Model['forwards'][number]): ForwardSummary_Serialize => ({
  ...f, host_label: m.hosts.find(h => h.id === f.host_id)?.label ?? 'Unknown host',
})

export const data: Pick<Handlers, Keys> = {
  snippetsList: (_a, { m }) => ({ packages: m.packages, snippets: m.snippets }),
  snippetSave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.label ?? '').trim() || !String(i.script ?? '').trim()) fail('validation', 'A snippet needs a name and a script.')
    const next = { label: i.label, script: i.script, package_id: i.package_id ?? null, target_host_ids: i.target_host_ids ?? [] }
    const row = i.id ? m.snippets.find(s => s.id === i.id) : undefined
    if (row) Object.assign(row, next)
    else m.snippets.push({ id: id('snippet'), ...next })
    return null
  },
  snippetDelete: (a, { m }) => {
    m.snippets = m.snippets.filter(s => s.id !== a.id)
    return null
  },
  snippetsMove: (a, { m }) => {
    m.snippets.filter(s => a.ids.includes(s.id)).forEach((s) => { s.package_id = a.packageId })
    return null
  },
  packageSave: (a, { m, id }) => {
    if (!String(a.label ?? '').trim()) fail('validation', 'Give the package a name.')
    const row = a.id ? m.packages.find(p => p.id === a.id) : undefined
    if (row) row.label = a.label
    else m.packages.push({ id: id('package'), label: a.label })
    return null
  },
  packageDelete: (a, { m }) => {
    m.packages = m.packages.filter(p => p.id !== a.id)
    m.snippets.forEach((s) => { if (s.package_id === a.id) s.package_id = null })
    return null
  },

  forwardsList: (_a, { m }) => m.forwards.map(f => forwardSummary(m, f)),
  forwardSave: (a, { m, id }) => {
    const i = a.input
    if (!i.bind_port || i.bind_port < 1 || i.bind_port > 65535) fail('validation', 'The port must be between 1 and 65535.')
    const next = {
      host_id: i.host_id, label: i.label, kind: i.kind, bind_address: i.bind_address, bind_port: i.bind_port,
      dest_address: i.dest_address ?? null, dest_port: i.dest_port ?? null, autostart: !!i.autostart,
    }
    let row = i.id ? m.forwards.find(f => f.id === i.id) : undefined
    if (row) Object.assign(row, next)
    else {
      row = { id: id('forward'), ...next, state: 'stopped', connections: 0, error: null }
      m.forwards.push(row)
    }
    return forwardSummary(m, row)
  },
  forwardDelete: (a, { m }) => {
    m.forwards = m.forwards.filter(f => f.id !== a.id)
    return null
  },
  forwardStart: (a, { m, tag, emit }) => {
    const f = m.forwards.find(x => x.id === a.id)
    if (!f) fail('not_found', 'That rule is gone.')
    tag(`forward-start:${f.id}`)
    // Like Rust: binding the port fails when something else already listens there, and the rule shows an error.
    if (m.forwards.some(x => x.id !== f.id && x.bind_port === f.bind_port && x.state === 'active')) {
      Object.assign(f, { state: 'error', error: `Port ${f.bind_port} is already in use. Try ${f.bind_port + 10000}.` })
      emit('tunnel-changed', { id: f.id, state: 'error', connections: 0, error: f.error })
      return null
    }
    f.state = 'starting'
    f.error = null
    emit('tunnel-changed', { id: f.id, state: 'starting', connections: 0, error: null })
    return null
  },
  forwardStop: (a, { m }) => {
    const f = m.forwards.find(x => x.id === a.id)
    if (f) Object.assign(f, { state: 'stopped', connections: 0, error: null })
  },
  forwardRespond: (a, { tag }) => {
    tag(`forward-respond:${a.promptId}`)
    return null
  },

  workspacesList: (_a, { m }) => m.templates,
  workspaceSave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.name ?? '').trim()) fail('validation', 'Give the template a name.')
    const row = i.id ? m.templates.find(t => t.id === i.id) : undefined
    if (row) Object.assign(row, { name: i.name, mode: i.mode, layout: i.layout, panes: i.panes })
    else m.templates.push({ id: id('template'), name: i.name, mode: i.mode, layout: i.layout, panes: i.panes })
    return row?.id ?? m.templates[m.templates.length - 1]!.id
  },
  workspaceDelete: (a, { m }) => {
    m.templates = m.templates.filter(t => t.id !== a.id)
    return null
  },

  logsList: (_a, { m }) => m.logs,
  logBookmark: (a, { m }) => {
    const l = m.logs.find(x => x.id === a.id)
    if (l) Object.assign(l, { bookmarked: a.bookmarked, comment: a.comment })
    return null
  },
  logDelete: (a, { m }) => {
    m.logs = m.logs.filter(l => l.id !== a.id)
    return null
  },
  logReplay: (a, { channel, sendBytes }) => {
    sendBytes(channel(a.data), new TextEncoder().encode('Last login: Thu Nov 6 14:02:11 2026\r\n$ ls\r\nnotes.txt  deploy.sh\r\n$ '))
    return null
  },
}

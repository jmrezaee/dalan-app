import type { AgentConfig_Serialize, EnvConfig_Serialize, ImportPreview_Serialize, ProxyConfig_Serialize, Themes_Serialize } from '../../../app/types/bindings'
import { groupChain, groupSummary, hostSummary, themeFromGroups, type GroupRec, type HostRec } from '../model'
import { emptyProfile } from '../model'
import { fail, type Handlers } from '../types'

type Keys = 'hostsTree' | 'hostGet' | 'hostSave' | 'hostDelete' | 'hostDuplicate' | 'hostSetProtocol' | 'serialGet' | 'serialSet' | 'serialPorts'
  | 'groupSave' | 'groupDelete' | 'hostsMove' | 'tagsList' | 'tagDelete' | 'recentConnections' | 'terminalThemesList' | 'terminalThemeSet'
  | 'envGet' | 'envSet' | 'proxyGet' | 'proxySet' | 'agentGet' | 'agentSet' | 'importPreview' | 'importApply'

/** The tags in use: a name is a tag while some host carries it. */
const tagNames = (hosts: HostRec[]) => [...new Set(hosts.flatMap(h => h.tags))].sort()

export const hosts: Pick<Handlers, Keys> = {
  hostsTree: (_a, { m }) => ({ groups: m.groups.map(g => groupSummary(m, g)), hosts: m.hosts.map(h => hostSummary(m, h)) }),
  hostGet: (a, { m }) => {
    const h = m.hosts.find(x => x.id === a.id)
    return h ? hostSummary(m, h) : fail('not_found', 'That host is gone.')
  },
  hostSave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.address ?? '').trim()) fail('validation', 'Enter an address.')
    if (i.port != null && (i.port < 1 || i.port > 65535)) fail('validation', 'The port must be between 1 and 65535.')
    const own = {
      label: String(i.label || i.address).trim(), address: String(i.address).trim(), port: i.port ?? null, username: i.username ?? null,
      group_id: i.group_id ?? null, key_id: i.key_id ?? null, identity_id: i.identity_id ?? null, startup_snippet_id: i.startup_snippet_id ?? null,
      jump_mode: i.jump_mode, jump_host_ids: i.jump_host_ids ?? [], tags: i.tags ?? [],
    }
    let h = i.id ? m.hosts.find(x => x.id === i.id) : undefined
    if (i.id && !h) fail('not_found', 'That host is gone.')
    if (!h) {
      h = { ...emptyProfile(), protocol: 'ssh', os_hint: null, last_connected_at: null, serial: { baud: 9600, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' }, ...own, id: id('host') }
      m.hosts.push(h)
    }
    else Object.assign(h, own)
    // The password goes in, and only a flag comes out.
    if (i.clear_password) h.password = false
    else if (i.password) h.password = true
    return hostSummary(m, h)
  },
  hostDelete: (a, { m }) => {
    if (m.hosts.some(h => h.jump_host_ids.includes(a.id))) fail('in_use', 'Another host jumps through this one.')
    m.hosts = m.hosts.filter(h => h.id !== a.id)
    return null
  },
  hostDuplicate: (a, { m, id }) => {
    const h = m.hosts.find(x => x.id === a.id)
    if (!h) fail('not_found', 'That host is gone.')
    const copy: HostRec = { ...h, id: id('host'), label: `${h.label} copy`, tags: [...h.tags], jump_host_ids: [...h.jump_host_ids], env_vars: h.env_vars.map(v => ({ ...v })) }
    m.hosts.push(copy)
    return hostSummary(m, copy)
  },
  hostSetProtocol: (a, { m }) => {
    const h = m.hosts.find(x => x.id === a.id)
    if (h) h.protocol = a.protocol
    return null
  },
  serialGet: (a, { m }) => m.hosts.find(h => h.id === a.hostId)?.serial ?? fail('not_found', 'That host is gone.'),
  serialSet: (a, { m }) => {
    const h = m.hosts.find(x => x.id === a.hostId)
    if (h) h.serial = a.config
    return null
  },
  serialPorts: () => ['/dev/ttyUSB0', '/dev/ttyACM0'],

  groupSave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.label ?? '').trim()) fail('validation', 'Give the group a name.')
    // A group may not sit under itself or its own children.
    if (i.id && i.parent_id && (i.parent_id === i.id || groupChain(m, i.parent_id).some(g => g.id === i.id))) fail('validation', 'A group can\'t go inside itself.')
    const own = {
      label: String(i.label).trim(), parent_id: i.parent_id ?? null, port: i.port ?? null, username: i.username ?? null,
      key_id: i.key_id ?? null, identity_id: i.identity_id ?? null, startup_snippet_id: i.startup_snippet_id ?? null,
      jump_mode: i.jump_mode, jump_host_ids: i.jump_host_ids ?? [],
    }
    let g = i.id ? m.groups.find(x => x.id === i.id) : undefined
    if (!g) {
      g = { ...emptyProfile(), ...own, id: id('group') } as GroupRec
      m.groups.push(g)
    }
    else Object.assign(g, own)
    if (i.clear_password) g.password = false
    else if (i.password) g.password = true
    return groupSummary(m, g)
  },
  groupDelete: (a, { m }) => {
    const g = m.groups.find(x => x.id === a.id)
    if (!g) return null
    // Children move up one level.
    m.groups.filter(x => x.parent_id === g.id).forEach((x) => { x.parent_id = g.parent_id })
    m.hosts.filter(h => h.group_id === g.id).forEach((h) => { h.group_id = g.parent_id })
    m.groups = m.groups.filter(x => x.id !== g.id)
    return null
  },
  hostsMove: (a, { m }) => {
    m.hosts.filter(h => a.ids.includes(h.id)).forEach((h) => { h.group_id = a.groupId })
    return null
  },
  tagsList: (_a, { m }) => tagNames(m.hosts).map(name => ({ id: `tag-${name}`, name, hosts: m.hosts.filter(h => h.tags.includes(name)).length })),
  tagDelete: (a, { m }) => {
    const name = String(a.id).replace(/^tag-/, '')
    m.hosts.forEach((h) => { h.tags = h.tags.filter(t => t !== name) })
    return null
  },
  recentConnections: (a, { m }) => m.hosts
    .filter(h => h.last_connected_at)
    .sort((x, y) => (y.last_connected_at ?? 0) - (x.last_connected_at ?? 0))
    .slice(0, a.limit)
    .map(h => ({ host_id: h.id, label: h.label, address: h.address, connected_at: h.last_connected_at })),

  terminalThemesList: (_a, { m }): Themes_Serialize => ({
    hosts: m.hosts.map(h => ({ id: h.id, own: h.theme, ...themeFromGroups(m, h.group_id) })),
    groups: m.groups.map(g => ({ id: g.id, own: g.theme, ...themeFromGroups(m, g.parent_id) })),
  }),
  terminalThemeSet: (a, { m }) => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    if (row) row.theme = a.theme
    return null
  },

  envGet: (a, { m }): EnvConfig_Serialize => {
    const own = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    const parent = a.kind === 'group' ? m.groups.find(g => g.id === a.id)?.parent_id ?? null : m.hosts.find(h => h.id === a.id)?.group_id ?? null
    const inherited = groupChain(m, parent).flatMap(g => g.env_vars.map(v => ({ name: v.name, value: v.value, from: g.label })))
    return { mode: own?.env_mode ?? 'inherit', vars: own?.env_vars ?? [], inherited }
  },
  envSet: (a, { m }) => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    if (row) Object.assign(row, { env_mode: a.mode, env_vars: a.vars })
    return null
  },
  proxyGet: (a, { m }): ProxyConfig_Serialize => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    const parent = a.kind === 'group' ? m.groups.find(g => g.id === a.id)?.parent_id ?? null : m.hosts.find(h => h.id === a.id)?.group_id ?? null
    const from = groupChain(m, parent).find(g => g.proxy.kind !== 'inherit')
    return { own: row?.proxy ?? { kind: 'inherit', host: '', port: null }, inherited: from ? from.proxy.kind : null, inherited_from: from?.label ?? null }
  },
  proxySet: (a, { m }) => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    if (row) row.proxy = { kind: a.choice.kind, host: a.choice.host, port: a.choice.port ?? null }
    return null
  },
  agentGet: (a, { m }): AgentConfig_Serialize => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    const parent = a.kind === 'group' ? m.groups.find(g => g.id === a.id)?.parent_id ?? null : m.hosts.find(h => h.id === a.id)?.group_id ?? null
    const from = groupChain(m, parent).find(g => g.agent.kind !== 'inherit')
    return { own: row?.agent ?? { kind: 'inherit' }, inherited: from ? from.agent.kind === 'forward' : null, inherited_from: from?.label ?? null, available: true }
  },
  agentSet: (a, { m }) => {
    const row = a.kind === 'group' ? m.groups.find(g => g.id === a.id) : m.hosts.find(h => h.id === a.id)
    if (row) row.agent = a.choice
    return null
  },

  // Import reads a file in Rust. The fake answers with one canned file: a new host, a duplicate, and a warning.
  importPreview: (): ImportPreview_Serialize => ({
    rows: [
      { label: 'api', address: 'api.example.com', port: 2200, username: 'deploy', group: null, tags: [], jump: ['bastion'], duplicate: false, warnings: [] },
      { label: 'web-1', address: '10.0.0.1', port: null, username: 'me', group: null, tags: [], jump: [], duplicate: true, warnings: [] },
      { label: 'odd', address: '10.9.9.9', port: null, username: null, group: null, tags: [], jump: [], duplicate: false, warnings: ['ProxyCommand was ignored.'] },
    ],
    warnings: ['An Include line was not followed.'],
  }),
  importApply: (a, { m, id, tag }) => {
    tag(`import:${a.rows.length}`)
    let jumpsMissing = 0
    for (const r of a.rows) {
      m.hosts.push({ ...emptyProfile(), id: id('host'), label: r.label, address: r.address, port: r.port ?? null, username: r.username ?? null, group_id: a.groupId ?? null, tags: r.tags ?? [], protocol: 'ssh', os_hint: null, last_connected_at: null, serial: { baud: 9600, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' } })
      jumpsMissing += (r.jump ?? []).filter((j: string) => !m.hosts.some(h => h.label === j)).length
    }
    return { created: a.rows.length, skipped: 0, jumps_missing: jumpsMissing }
  },
}

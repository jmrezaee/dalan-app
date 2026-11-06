// The in-memory vault the fake backend answers from. It holds what Rust's SQLite would, and no secret:
// a password is only a flag (`password: true`), like the real `has_password`.
import type {
  AgentChoice, EnvVar, ForwardKind, Inherited_Serialize, KeySummary_Serialize, KnownHost_Serialize, LogSummary_Serialize,
  PackageSummary, ProxyChoice_Serialize, SerialConfig, TemplateSummary, TunnelState, HostSummary_Serialize, GroupSummary_Serialize,
} from '../../app/types/bindings'

export interface Profile {
  username: string | null
  port: number | null
  password: boolean
  key_id: string | null
  identity_id: string | null
  startup_snippet_id: string | null
  jump_mode: string
  jump_host_ids: string[]
  theme: string | null
  env_mode: string
  env_vars: EnvVar[]
  proxy: ProxyChoice_Serialize
  agent: AgentChoice
}

export interface HostRec extends Profile {
  id: string
  label: string
  address: string
  group_id: string | null
  tags: string[]
  protocol: string
  os_hint: string | null
  last_connected_at: number | null
  serial: SerialConfig
}

export interface GroupRec extends Profile {
  id: string
  parent_id: string | null
  label: string
}

export interface IdentityRec { id: string, label: string, username: string, password: boolean, key_id: string | null }
export interface SnippetRec { id: string, label: string, script: string, package_id: string | null, target_host_ids: string[] }
export interface ForwardRec { id: string, host_id: string, label: string, kind: ForwardKind, bind_address: string, bind_port: number, dest_address: string | null, dest_port: number | null, autostart: boolean, state: TunnelState, connections: number, error: string | null }
export interface FsNode { name: string, kind: 'file' | 'dir', size: number, mtime: number, mode: number, children?: FsNode[] }

export interface SessionRec { events: number, data: number | null, host: string, closed: boolean }

/** The data a scenario seeds. Everything optional falls back to empty. */
export interface Seed {
  vault?: { exists: boolean, unlocked: boolean, autoLockMinutes?: number }
  groups?: (Partial<GroupRec> & { id: string, label: string })[]
  hosts?: (Partial<HostRec> & { id: string, label: string, address: string })[]
  keys?: KeySummary_Serialize[]
  identities?: IdentityRec[]
  snippets?: SnippetRec[]
  packages?: PackageSummary[]
  forwards?: ForwardRec[]
  knownHosts?: KnownHost_Serialize[]
  logs?: LogSummary_Serialize[]
  templates?: TemplateSummary[]
  history?: { id: string, command: string, ran_at: number | null, host_id: string | null }[]
  settings?: Record<string, string>
  /** Server file trees by host id; "local" is this computer. */
  files?: Record<string, FsNode[]>
  now?: number
}

export const emptyProfile = (): Profile => ({
  username: null, port: null, password: false, key_id: null, identity_id: null, startup_snippet_id: null,
  jump_mode: 'inherit', jump_host_ids: [], theme: null, env_mode: 'inherit', env_vars: [],
  proxy: { kind: 'inherit', host: '', port: null }, agent: { kind: 'inherit' },
})

export interface Model {
  now: number
  vault: { exists: boolean, unlocked: boolean, autoLockMinutes: number, wrongTries: number, cooldownUntil: number }
  groups: GroupRec[]
  hosts: HostRec[]
  keys: KeySummary_Serialize[]
  identities: IdentityRec[]
  snippets: SnippetRec[]
  packages: PackageSummary[]
  forwards: ForwardRec[]
  knownHosts: KnownHost_Serialize[]
  logs: LogSummary_Serialize[]
  templates: TemplateSummary[]
  history: { id: string, command: string, ran_at: number | null, host_id: string | null }[]
  settings: Record<string, string>
  files: Record<string, FsNode[]>
  openWith: Record<string, string>
  ai: { model: string, base_url: string, has_key: boolean }
  sessions: Map<string, SessionRec>
  /** Open SFTP connections: sftp id -> host id. */
  sftp: Map<string, string>
  /** Running transfers and edits, with the channel their events go to. */
  transfers: Map<string, { events: number, items: { src_endpoint: string, src_path: string, dst_endpoint: string, dst_dir: string }[], pending: number | null, applyAll: string | null }>
  edits: Map<string, { events: number, endpoint: string, path: string }>
  /** Transfers finish by themselves unless a test takes over with `fake.manualTransfers()`. */
  manualTransfers: boolean
  /** The next `edit_upload` says the server file changed. */
  editRemoteChanged: boolean
  /** Host key the next connect to a host presents; `rotateHostKey` changes it. */
  hostKeys: Map<string, { key_type: string, fingerprint: string }>
  /** Everything a test may want to look at afterwards. */
  written: { id: string, text: string }[]
  responses: { id: string, prompt: string, answer: unknown }[]
  calls: string[]
  unhandled: string[]
  /** Commands that fail once with this error (`fake.failNext`). */
  failNext: Map<string, { code: string, message: string }>
}

export function createModel(seed: Seed): Model {
  const now = seed.now ?? Date.UTC(2026, 10, 6, 15, 0, 0)
  return {
    now,
    vault: { exists: seed.vault?.exists ?? true, unlocked: seed.vault?.unlocked ?? false, autoLockMinutes: seed.vault?.autoLockMinutes ?? 15, wrongTries: 0, cooldownUntil: 0 },
    groups: (seed.groups ?? []).map((g): GroupRec => ({ ...emptyProfile(), parent_id: null, ...g })),
    hosts: (seed.hosts ?? []).map((h): HostRec => ({ ...emptyProfile(), group_id: null, tags: [], protocol: 'ssh', os_hint: null, last_connected_at: null, serial: { baud: 9600, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' }, ...h })),
    keys: seed.keys ?? [],
    identities: seed.identities ?? [],
    snippets: seed.snippets ?? [],
    packages: seed.packages ?? [],
    forwards: seed.forwards ?? [],
    knownHosts: seed.knownHosts ?? [],
    logs: seed.logs ?? [],
    templates: seed.templates ?? [],
    history: seed.history ?? [],
    settings: seed.settings ?? {},
    files: seed.files ?? {},
    openWith: {},
    ai: { model: 'gpt-4o-mini', base_url: 'https://api.example.com/v1', has_key: false },
    sessions: new Map(),
    sftp: new Map(),
    transfers: new Map(),
    edits: new Map(),
    manualTransfers: false,
    editRemoteChanged: false,
    hostKeys: new Map(),
    written: [],
    responses: [],
    calls: [],
    unhandled: [],
    failNext: new Map(),
  }
}

// ---- inheritance: host -> group -> parent group (the same order as Rust)

const groupChain = (m: Model, groupId: string | null): GroupRec[] => {
  const out: GroupRec[] = []
  for (let id = groupId, guard = 0; id && guard < 20; guard++) {
    const g = m.groups.find(x => x.id === id)
    if (!g) break
    out.push(g)
    id = g.parent_id
  }
  return out
}

const firstFrom = <T>(chain: GroupRec[], pick: (g: GroupRec) => T | null | undefined) => {
  for (const g of chain) {
    const v = pick(g)
    if (v !== null && v !== undefined) return { value: v, group: g.label }
  }
  return null
}

function inheritedFor(m: Model, own: Profile, chain: GroupRec[]): Inherited_Serialize {
  const jump = own.jump_mode === 'inherit' ? chain.find(g => g.jump_mode === 'custom') : undefined
  const startup = firstFrom(chain, g => g.startup_snippet_id)
  const key = firstFrom(chain, g => g.key_id)
  return {
    username: firstFrom(chain, g => g.username),
    port: firstFrom(chain, g => g.port),
    password: chain.find(g => g.password)?.label ?? null,
    key: key ? { value: m.keys.find(k => k.id === key.value)?.label ?? key.value, group: key.group } : null,
    jump: jump ? { host_ids: jump.jump_host_ids, group: jump.label } : null,
    startup_snippet: startup ? { value: m.snippets.find(s => s.id === startup.value)?.label ?? startup.value, group: startup.group } : null,
  }
}

export function hostSummary(m: Model, h: HostRec): HostSummary_Serialize {
  const chain = groupChain(m, h.group_id)
  const inh = inheritedFor(m, h, chain)
  const identity = m.identities.find(i => i.id === h.identity_id)
  return {
    id: h.id, label: h.label, address: h.address, port: h.port, username: h.username, has_password: h.password,
    group_id: h.group_id, tags: h.tags,
    effective_username: h.username ?? identity?.username ?? inh.username?.value ?? null,
    effective_port: h.port ?? inh.port?.value ?? 22,
    effective_has_password: h.password || !!identity?.password || !!inh.password,
    key_id: h.key_id, identity_id: h.identity_id,
    effective_key_id: h.key_id ?? identity?.key_id ?? chain.find(g => g.key_id)?.key_id ?? null,
    startup_snippet_id: h.startup_snippet_id, jump_mode: h.jump_mode, jump_host_ids: h.jump_host_ids,
    effective_jump_host_ids: h.jump_mode === 'custom' ? h.jump_host_ids : h.jump_mode === 'direct' ? [] : inh.jump?.host_ids ?? [],
    inherited: inh, last_connected_at: h.last_connected_at, os_hint: h.os_hint, protocol: h.protocol,
  }
}

export function groupSummary(m: Model, g: GroupRec): GroupSummary_Serialize {
  const chain = groupChain(m, g.parent_id)
  return {
    id: g.id, parent_id: g.parent_id, label: g.label, username: g.username, port: g.port, has_password: g.password,
    key_id: g.key_id, identity_id: g.identity_id, startup_snippet_id: g.startup_snippet_id, jump_mode: g.jump_mode,
    jump_host_ids: g.jump_host_ids, host_count: m.hosts.filter(h => h.group_id === g.id).length,
    group_count: m.groups.filter(x => x.parent_id === g.id).length, inherited: inheritedFor(m, g, chain),
  }
}

/** The theme a host or group would get from the groups above it, and the group that sets it. */
export function themeFromGroups(m: Model, groupId: string | null) {
  const g = groupChain(m, groupId).find(x => x.theme)
  return { inherited: g?.theme ?? null, inherited_from: g?.label ?? null }
}
export { groupChain }

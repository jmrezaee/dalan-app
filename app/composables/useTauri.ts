// The only place that talks to Rust. Stores call these wrappers, never `invoke` directly.
import { Channel } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { commands } from '~/types/bindings'
import type { AuthMethod, AiSettings, SerialConfig, AgentChoice, AppError, ProxyChoice, EnvVar, EditApp, EditEvent, TemplateInput, Choice, TransferEvent, TransferItem, ForwardInput, SnippetInput, GenerateInput, GroupInput, HostInput, IdentityInput, ImportInput, ImportKind, ImportRow, KeyUpdate, PromptAnswer, RawBytes, SessionEvent } from '~/types/bindings'

type Result<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError }

/** Thrown for any failed command. Carries the `{ code, message }` shape from Rust. */
export class CommandError extends Error {
  constructor(public readonly code: AppError['code'], message: string) {
    super(message)
    this.name = 'CommandError'
  }
}

/** Turns the generated `{ status, data | error }` result into a plain value or a thrown CommandError. */
export async function unwrap<T>(call: Promise<Result<T>>): Promise<T> {
  const res = await call
  if (res.status === 'error') throw new CommandError(res.error.code, res.error.message)
  return res.data
}

export function useTauri() {
  return {
    appInfo: () => unwrap(commands.appInfo()),
    quitApp: () => commands.quitApp(),
    quitCancel: () => commands.quitCancel(),
    linksTake: () => commands.linksTake(),
    systemIntegrationGet: () => unwrap(commands.systemIntegrationGet()),
    systemIntegrationSet: (sshLinks: boolean | null, autostart: boolean | null) => unwrap(commands.systemIntegrationSet(sshLinks, autostart)),
    openDefaultApps: () => unwrap(commands.openDefaultApps()),
    vaultStatus: () => unwrap(commands.vaultStatus()),
    vaultCreate: (masterPassword: string) => unwrap(commands.vaultCreate(masterPassword)),
    vaultUnlock: (masterPassword: string) => unwrap(commands.vaultUnlock(masterPassword)),
    vaultLock: () => unwrap(commands.vaultLock()),
    vaultChangePassword: (oldPassword: string, newPassword: string) => unwrap(commands.vaultChangePassword(oldPassword, newPassword)),
    vaultTouch: () => commands.vaultTouch(),
    hostsTree: () => unwrap(commands.hostsTree()),
    hostGet: (id: string) => unwrap(commands.hostGet(id)),
    hostSave: (input: HostInput) => unwrap(commands.hostSave(input)),
    serialGet: (hostId: string) => unwrap(commands.serialGet(hostId)),
    serialSet: (hostId: string, config: SerialConfig) => unwrap(commands.serialSet(hostId, config)),
    serialPorts: () => unwrap(commands.serialPorts()),
    hostSetProtocol: (id: string, protocol: string) => unwrap(commands.hostSetProtocol(id, protocol)),
    hostDelete: (id: string) => unwrap(commands.hostDelete(id)),
    hostDuplicate: (id: string) => unwrap(commands.hostDuplicate(id)),
    groupSave: (input: GroupInput) => unwrap(commands.groupSave(input)),
    groupDelete: (id: string) => unwrap(commands.groupDelete(id)),
    hostsMove: (ids: string[], groupId: string | null) => unwrap(commands.hostsMove(ids, groupId)),
    tagsList: () => unwrap(commands.tagsList()),
    tagDelete: (id: string) => unwrap(commands.tagDelete(id)),
    recentConnections: (limit: number) => unwrap(commands.recentConnections(limit)),
    /** Terminal output arrives as raw bytes on its own channel (an ArrayBuffer, not JSON). */
    sessionOpen: (hostId: string, cols: number, rows: number, onEvent: (e: SessionEvent) => void, onData: (b: Uint8Array) => void) => {
      const events = new Channel<SessionEvent>()
      events.onmessage = onEvent
      const data = new Channel<ArrayBuffer>()
      data.onmessage = m => onData(new Uint8Array(m))
      return unwrap(commands.sessionOpen(hostId, cols, rows, events as never, data as unknown as Channel<RawBytes>))
    },
    localSessionOpen: (cols: number, rows: number, onEvent: (e: SessionEvent) => void, onData: (b: Uint8Array) => void) => {
      const events = new Channel<SessionEvent>()
      events.onmessage = onEvent
      const data = new Channel<ArrayBuffer>()
      data.onmessage = m => onData(new Uint8Array(m))
      return unwrap(commands.localSessionOpen(cols, rows, events as never, data as unknown as Channel<RawBytes>))
    },
    sessionWrite: (id: string, bytes: Uint8Array) => unwrap(commands.sessionWrite(id, Array.from(bytes))),
    sessionResize: (id: string, cols: number, rows: number) => unwrap(commands.sessionResize(id, cols, rows)),
    sessionAck: (id: string, bytes: number) => unwrap(commands.sessionAck(id, bytes)),
    sessionRespond: (id: string, promptId: string, answer: PromptAnswer) => unwrap(commands.sessionRespond(id, promptId, answer)),
    sessionClose: (id: string) => commands.sessionClose(id),
    sessionReconnect: (id: string) => unwrap(commands.sessionReconnect(id)),
    /** "Try another way": reconnect and sign in with only this method. */
    sessionReconnectWith: (id: string, method: AuthMethod) => unwrap(commands.sessionReconnectWith(id, method)),
    /** The caller picks `openId` so it can answer host-key and password questions while this runs. */
    sftpOpen: (openId: string, hostId: string, onEvent: (e: SessionEvent) => void) => {
      const events = new Channel<SessionEvent>()
      events.onmessage = onEvent
      return unwrap(commands.sftpOpen(openId, hostId, events as never))
    },
    sftpRespond: (openId: string, promptId: string, answer: PromptAnswer) => unwrap(commands.sftpRespond(openId, promptId, answer)),
    sftpClose: (sftpId: string) => commands.sftpClose(sftpId),
    /** Asks the OS to choose one file. Returns its path only; Rust reads the file. `null` when cancelled. */
    pickFile: async (): Promise<string | null> => {
      const p = await openDialog({ multiple: false, directory: false })
      return typeof p === 'string' ? p : null
    },
    fsHome: (endpoint: string) => unwrap(commands.fsHome(endpoint)),
    fsRealpath: (endpoint: string, path: string) => unwrap(commands.fsRealpath(endpoint, path)),
    fsList: (endpoint: string, path: string, showHidden: boolean) => unwrap(commands.fsList(endpoint, path, showHidden)),
    fsMkdir: (endpoint: string, path: string) => unwrap(commands.fsMkdir(endpoint, path)),
    fsRename: (endpoint: string, from: string, to: string) => unwrap(commands.fsRename(endpoint, from, to)),
    fsRemove: (endpoint: string, path: string) => unwrap(commands.fsRemove(endpoint, path)),
    fsChmod: (endpoint: string, path: string, mode: number) => unwrap(commands.fsChmod(endpoint, path, mode)),
    transferStart: (items: TransferItem[], onEvent: (e: TransferEvent) => void) => {
      const events = new Channel<TransferEvent>()
      events.onmessage = onEvent
      return unwrap(commands.transferStart(items, events as never))
    },
    transferResolve: (id: string, choice: Choice, applyToAll: boolean) => unwrap(commands.transferResolve(id, choice, applyToAll)),
    transferCancel: (id: string) => unwrap(commands.transferCancel(id)),
    editOpen: (endpoint: string, path: string, withApp: EditApp, onEvent: (e: EditEvent) => void) => {
      const events = new Channel<EditEvent>()
      events.onmessage = onEvent
      return unwrap(commands.editOpen(endpoint, path, withApp, events as never))
    },
    editApps: () => unwrap(commands.editApps()),
    editAppForget: (ext: string) => unwrap(commands.editAppForget(ext)),
    editUpload: (editId: string, force: boolean) => unwrap(commands.editUpload(editId, force)),
    editClose: (editId: string) => commands.editClose(editId),
    updateCheck: () => unwrap(commands.updateCheck()),
    updateInstall: () => unwrap(commands.updateInstall()),
    diagnosticsText: () => unwrap(commands.diagnosticsText()),
    historyList: (hostId: string | null, query: string | null, limit: number) => unwrap(commands.historyList(hostId, query, limit)),
    historyClear: (hostId: string | null, all: boolean) => unwrap(commands.historyClear(hostId, all)),
    logsList: () => unwrap(commands.logsList()),
    logBookmark: (id: string, bookmarked: boolean, comment: string | null) => unwrap(commands.logBookmark(id, bookmarked, comment)),
    logDelete: (id: string) => unwrap(commands.logDelete(id)),
    /** Sends the log's decrypted bytes to `onData` in order; resolves when all have been sent. */
    logReplay: (id: string, onData: (b: Uint8Array) => void) => {
      const data = new Channel<ArrayBuffer>()
      data.onmessage = m => onData(new Uint8Array(m))
      return unwrap(commands.logReplay(id, data as unknown as Channel<RawBytes>))
    },
    aiGet: () => unwrap(commands.aiGet()),
    aiSave: (input: AiSettings) => unwrap(commands.aiSave(input)),
    aiSuggest: (request: string, hostId: string | null) => unwrap(commands.aiSuggest(request, hostId)),
    agentGet: (kind: 'host' | 'group', id: string) => unwrap(commands.agentGet(kind, id)),
    agentSet: (kind: 'host' | 'group', id: string, choice: AgentChoice) => unwrap(commands.agentSet(kind, id, choice)),
    proxyGet: (kind: 'host' | 'group', id: string) => unwrap(commands.proxyGet(kind, id)),
    proxySet: (kind: 'host' | 'group', id: string, choice: ProxyChoice) => unwrap(commands.proxySet(kind, id, choice)),
    envGet: (kind: 'host' | 'group', id: string) => unwrap(commands.envGet(kind, id)),
    envSet: (kind: 'host' | 'group', id: string, mode: string, vars: EnvVar[]) => unwrap(commands.envSet(kind, id, mode, vars)),
    terminalThemesList: () => unwrap(commands.terminalThemesList()),
    terminalThemeSet: (kind: 'host' | 'group', id: string, theme: string | null) => unwrap(commands.terminalThemeSet(kind, id, theme)),
    importPreview: (kind: ImportKind, path: string | null) => unwrap(commands.importPreview(kind, path)),
    importApply: (rows: ImportRow[], groupId: string | null) => unwrap(commands.importApply(rows, groupId)),
    workspacesList: () => unwrap(commands.workspacesList()),
    workspaceSave: (input: TemplateInput) => unwrap(commands.workspaceSave(input)),
    workspaceDelete: (id: string) => unwrap(commands.workspaceDelete(id)),
    forwardsList: () => unwrap(commands.forwardsList()),
    forwardSave: (input: ForwardInput) => unwrap(commands.forwardSave(input)),
    forwardDelete: (id: string) => unwrap(commands.forwardDelete(id)),
    /** Host-key and password questions arrive on `onEvent`; answer with `forwardRespond`. */
    forwardStart: (id: string, onEvent: (e: SessionEvent) => void) => {
      const events = new Channel<SessionEvent>()
      events.onmessage = onEvent
      return unwrap(commands.forwardStart(id, events as never))
    },
    forwardStop: (id: string) => commands.forwardStop(id),
    forwardRespond: (id: string, promptId: string, answer: PromptAnswer) => unwrap(commands.forwardRespond(id, promptId, answer)),
    snippetsList: () => unwrap(commands.snippetsList()),
    snippetSave: (input: SnippetInput) => unwrap(commands.snippetSave(input)),
    snippetDelete: (id: string) => unwrap(commands.snippetDelete(id)),
    snippetsMove: (ids: string[], packageId: string | null) => unwrap(commands.snippetsMove(ids, packageId)),
    packageSave: (id: string | null, label: string) => unwrap(commands.packageSave(id, label)),
    packageDelete: (id: string) => unwrap(commands.packageDelete(id)),
    settingsGetAll: () => unwrap(commands.settingsGetAll()),
    settingsSet: (key: string, value: string) => unwrap(commands.settingsSet(key, value)),
    keysList: () => unwrap(commands.keysList()),
    keyGenerate: (input: GenerateInput) => unwrap(commands.keyGenerate(input)),
    keyImport: (input: ImportInput) => unwrap(commands.keyImport(input)),
    keySetCertificate: (id: string, certificate: string | null, path: string | null) => unwrap(commands.keySetCertificate(id, certificate, path)),
    keyUpdate: (input: KeyUpdate) => unwrap(commands.keyUpdate(input)),
    keyDelete: (id: string) => unwrap(commands.keyDelete(id)),
    identitiesList: () => unwrap(commands.identitiesList()),
    identitySave: (input: IdentityInput) => unwrap(commands.identitySave(input)),
    identityDelete: (id: string) => unwrap(commands.identityDelete(id)),
    /** S08. Progress and prompts arrive on `onEvent`; answer prompts with `sessionRespond`. */
    keyExportToHost: (keyId: string, hostId: string, dir: string, fileName: string, onEvent: (e: SessionEvent) => void) => {
      const events = new Channel<SessionEvent>()
      events.onmessage = onEvent
      return unwrap(commands.keyExportToHost(keyId, hostId, dir, fileName, events as never))
    },
    knownHostsList: () => unwrap(commands.knownHostsList()),
    knownHostDelete: (id: string) => unwrap(commands.knownHostDelete(id)),
  }
}

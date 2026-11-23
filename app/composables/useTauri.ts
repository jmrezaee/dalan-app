// The only place that talks to Rust. Stores call these wrappers, never `invoke` directly.
import { Channel } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { commands } from '~/types/bindings'
import type { CodeLanguage, MgAggRequest, MgDocOp, MgFindRequest, MgIndexOp, MgStage, AuthMethod, AiSettings, SerialConfig, AgentChoice, AppError, ProxyChoice, EnvVar, EditApp, EditEvent, TemplateInput, Choice, TransferEvent, TransferItem, ForwardInput, SnippetInput, GenerateInput, GroupInput, HostInput, IdentityInput, ImportInput, ImportKind, ImportRow, KeyUpdate, PromptAnswer, RawBytes, DatabaseInput_Deserialize, DbEvent_Serialize, SqlRunEvent_Serialize, RunMode, TxnAction, GridEdits_Deserialize, StructureOp_Deserialize, TableRef, ActivityView, SessionAction, ExportFormat, TransferProgress, PageRequest_Deserialize, SavedQueryInput_Deserialize, RedisBytes, RedisBulkAction, RedisBulkEvent, RedisDatabaseInput_Deserialize, RedisSubEvent_Serialize, RedisWriteMode, RedisWriteOp_Deserialize, RedisEvent_Serialize, SessionEvent } from '~/types/bindings'

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
    dbList: () => unwrap(commands.dbList()),
    dbSave: (input: DatabaseInput_Deserialize, masterPassword: string | null = null) => unwrap(commands.dbSave(input, masterPassword)),
    dbDelete: (id: string) => unwrap(commands.dbDelete(id)),
    dbTest: (input: DatabaseInput_Deserialize) => unwrap(commands.dbTest(input)),
    dbParseUrl: (url: string) => unwrap(commands.dbParseUrl(url)),
    dbEngines: () => commands.dbEngines(),
    savedList: () => unwrap(commands.savedList()),
    savedSave: (input: SavedQueryInput_Deserialize) => unwrap(commands.savedSave(input)),
    savedDelete: (id: string) => unwrap(commands.savedDelete(id)),
    dbOpen: (dbId: string, onEvent: (e: DbEvent_Serialize) => void) => {
      const events = new Channel<DbEvent_Serialize>()
      events.onmessage = onEvent
      return unwrap(commands.dbOpen(dbId, events as never))
    },
    dbRespond: (connId: string, promptId: string, answer: PromptAnswer) => unwrap(commands.dbRespond(connId, promptId, answer)),
    dbClose: (connId: string) => commands.dbClose(connId),
    mgTree: (connId: string, path: string[]) => unwrap(commands.mgTree(connId, path)),
    mgFind: (connId: string, request: MgFindRequest) => unwrap(commands.mgFind(connId, request)),
    mgCount: (connId: string, db: string, coll: string, filter: string) => unwrap(commands.mgCount(connId, db, coll, filter)),
    mgDocGet: (connId: string, db: string, coll: string, id: string) => unwrap(commands.mgDocGet(connId, db, coll, id)),
    mgDocPrepare: (connId: string, db: string, coll: string, op: MgDocOp) => unwrap(commands.mgDocPrepare(connId, db, coll, op)),
    mgDocCommit: (connId: string, pendingId: string, typedName: string | null = null) => unwrap(commands.mgDocCommit(connId, pendingId, typedName)),
    mgShell: (connId: string, db: string, line: string) => unwrap(commands.mgShell(connId, db, line)),
    mgShellCommit: (connId: string, pendingId: string, typedName: string | null) => unwrap(commands.mgShellCommit(connId, pendingId, typedName)),
    mgAggregate: (connId: string, request: MgAggRequest) => unwrap(commands.mgAggregate(connId, request)),
    mgAggPrepare: (connId: string, request: MgAggRequest) => unwrap(commands.mgAggPrepare(connId, request)),
    mgViewPrepare: (connId: string, db: string, coll: string, name: string, stages: MgStage[]) => unwrap(commands.mgViewPrepare(connId, db, coll, name, stages)),
    mgPipelineCode: (coll: string, stages: MgStage[], language: CodeLanguage) => unwrap(commands.mgPipelineCode(coll, stages, language)),
    mgSchema: (connId: string, db: string, coll: string, sample: number) => unwrap(commands.mgSchema(connId, db, coll, sample)),
    mgIndexes: (connId: string, db: string, coll: string) => unwrap(commands.mgIndexes(connId, db, coll)),
    mgIndexPrepare: (connId: string, db: string, coll: string, op: MgIndexOp) => unwrap(commands.mgIndexPrepare(connId, db, coll, op)),
    mgValidation: (connId: string, db: string, coll: string) => unwrap(commands.mgValidation(connId, db, coll)),
    mgValidationCheck: (connId: string, db: string, coll: string, validator: string) => unwrap(commands.mgValidationCheck(connId, db, coll, validator)),
    mgValidationPrepare: (connId: string, db: string, coll: string, validator: string, level: string, action: string) => unwrap(commands.mgValidationPrepare(connId, db, coll, validator, level, action)),
    mgDocDiscard: (connId: string, pendingId: string) => unwrap(commands.mgDocDiscard(connId, pendingId)),
    sqlObjects: (connId: string, path: string[]) => unwrap(commands.sqlObjects(connId, path)),
    sqlTableInfo: (connId: string, schema: string, table: string) => unwrap(commands.sqlTableInfo(connId, schema, table)),
    sqlTablePage: (connId: string, request: PageRequest_Deserialize) => unwrap(commands.sqlTablePage(connId, request)),
    sqlCount: (connId: string, schema: string, table: string, filter: string) => unwrap(commands.sqlCount(connId, schema, table, filter)),
    sqlCell: (connId: string, schema: string, table: string, key: { column: string, value: string }[], column: string) => unwrap(commands.sqlCell(connId, schema, table, key, column)),
    sqlRun: (connId: string, editorId: string, text: string, mode: RunMode, cursor: number | null, limit: number, timeoutMs: number, onEvent: (e: SqlRunEvent_Serialize) => void) => {
      const events = new Channel<SqlRunEvent_Serialize>()
      events.onmessage = onEvent
      return unwrap(commands.sqlRun(connId, editorId, text, mode, cursor, limit, timeoutMs, events as never))
    },
    sqlRunCommit: (connId: string, pendingId: string, typedName: string | null, limit: number, timeoutMs: number, onEvent: (e: SqlRunEvent_Serialize) => void) => {
      const events = new Channel<SqlRunEvent_Serialize>()
      events.onmessage = onEvent
      return unwrap(commands.sqlRunCommit(connId, pendingId, typedName, limit, timeoutMs, events as never))
    },
    sqlRunDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlRunDiscard(connId, pendingId)),
    sqlTxn: (connId: string, editorId: string, action: TxnAction) => unwrap(commands.sqlTxn(connId, editorId, action)),
    sqlGridPrepare: (connId: string, edits: GridEdits_Deserialize) => unwrap(commands.sqlGridPrepare(connId, edits)),
    sqlGridCommit: (connId: string, pendingId: string) => unwrap(commands.sqlGridCommit(connId, pendingId)),
    sqlGridDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlGridDiscard(connId, pendingId)),
    sqlStructure: (connId: string, schema: string, table: string) => unwrap(commands.sqlStructure(connId, schema, table)),
    sqlStructurePrepare: (connId: string, schema: string, table: string, ops: StructureOp_Deserialize[]) => unwrap(commands.sqlStructurePrepare(connId, schema, table, ops)),
    sqlStructureCommit: (connId: string, pendingId: string, typedName: string | null) => unwrap(commands.sqlStructureCommit(connId, pendingId, typedName)),
    sqlStructureDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlStructureDiscard(connId, pendingId)),
    sqlExplain: (connId: string, editorId: string, text: string, mode: RunMode, cursor: number | null, analyze: boolean, timeoutMs: number) => unwrap(commands.sqlExplain(connId, editorId, text, mode, cursor, analyze, timeoutMs)),
    sqlDiagram: (connId: string, tables: TableRef[], steps: number) => unwrap(commands.sqlDiagram(connId, tables, steps)),
    sqlSaveExport: (suggestedName: string, data: number[]) => unwrap(commands.sqlSaveExport(suggestedName, data)),
    sqlActivity: (connId: string, view: ActivityView) => unwrap(commands.sqlActivity(connId, view)),
    sqlSessionPrepare: (connId: string, pid: number, action: SessionAction) => unwrap(commands.sqlSessionPrepare(connId, pid, action)),
    sqlSessionCommit: (connId: string, pendingId: string, typedName: string | null) => unwrap(commands.sqlSessionCommit(connId, pendingId, typedName)),
    sqlSessionDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlSessionDiscard(connId, pendingId)),
    sqlExportTable: (connId: string, transferId: string, schema: string, table: string, filter: string, format: ExportFormat, header: boolean, onProgress: (p: TransferProgress) => void) => {
      const progress = new Channel<TransferProgress>()
      progress.onmessage = onProgress
      return unwrap(commands.sqlExportTable(connId, transferId, schema, table, filter, format, header, progress as never))
    },
    sqlTransferStop: (transferId: string) => unwrap(commands.sqlTransferStop(transferId)),
    sqlImportPick: (connId: string) => unwrap(commands.sqlImportPick(connId)),
    sqlImportPreview: (connId: string, schema: string, table: string, fileId: string, delimiterName: string, header: boolean) => unwrap(commands.sqlImportPreview(connId, schema, table, fileId, delimiterName, header)),
    sqlImportPrepare: (connId: string, schema: string, table: string, fileId: string, delimiterName: string, header: boolean, mapping: (string | null)[]) => unwrap(commands.sqlImportPrepare(connId, schema, table, fileId, delimiterName, header, mapping)),
    sqlImportCommit: (connId: string, pendingId: string) => unwrap(commands.sqlImportCommit(connId, pendingId)),
    sqlImportDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlImportDiscard(connId, pendingId)),
    sqlBackupCheck: (connId: string) => unwrap(commands.sqlBackupCheck(connId)),
    sqlBackupStart: (connId: string, transferId: string, onProgress: (p: TransferProgress) => void) => {
      const progress = new Channel<TransferProgress>()
      progress.onmessage = onProgress
      return unwrap(commands.sqlBackupStart(connId, transferId, progress as never))
    },
    sqlRestorePrepare: (connId: string) => unwrap(commands.sqlRestorePrepare(connId)),
    sqlRestoreCommit: (connId: string, pendingId: string, typedName: string | null, transferId: string, onProgress: (p: TransferProgress) => void) => {
      const progress = new Channel<TransferProgress>()
      progress.onmessage = onProgress
      return unwrap(commands.sqlRestoreCommit(connId, pendingId, typedName, transferId, progress as never))
    },
    sqlRestoreDiscard: (connId: string, pendingId: string) => unwrap(commands.sqlRestoreDiscard(connId, pendingId)),
    sqlCancel: (connId: string, editorId: string) => unwrap(commands.sqlCancel(connId, editorId)),
    sqlEditorClose: (connId: string, editorId: string) => unwrap(commands.sqlEditorClose(connId, editorId)),
    sqlCatalog: (connId: string, refresh: boolean) => unwrap(commands.sqlCatalog(connId, refresh)),
    sqlFormat: (text: string) => commands.sqlFormat(text),
    sqlHistory: (dbId: string) => unwrap(commands.sqlHistory(dbId)),
    sqlHistoryClear: (dbId: string) => unwrap(commands.sqlHistoryClear(dbId)),
    redisDbList: () => unwrap(commands.redisDbList()),
    redisDbSave: (input: RedisDatabaseInput_Deserialize, masterPassword: string | null = null) => unwrap(commands.redisDbSave(input, masterPassword)),
    redisDbDelete: (id: string) => unwrap(commands.redisDbDelete(id)),
    redisDbTest: (input: RedisDatabaseInput_Deserialize) => unwrap(commands.redisDbTest(input)),
    redisOpen: (dbId: string, onEvent: (e: RedisEvent_Serialize) => void) => {
      const events = new Channel<RedisEvent_Serialize>()
      events.onmessage = onEvent
      return unwrap(commands.redisOpen(dbId, events as never))
    },
    redisRespond: (connId: string, promptId: string, answer: PromptAnswer) => unwrap(commands.redisRespond(connId, promptId, answer)),
    redisSelect: (connId: string, index: number) => unwrap(commands.redisSelect(connId, index)),
    redisClose: (connId: string) => commands.redisClose(connId),
    redisScan: (connId: string, cursor: string, pattern: string, keyType: string | null, count: number) => unwrap(commands.redisScan(connId, cursor, pattern, keyType, count)),
    redisKeyInfo: (connId: string, key: RedisBytes) => unwrap(commands.redisKeyInfo(connId, key)),
    redisValuePage: (connId: string, key: RedisBytes, cursor: string, full: boolean) => unwrap(commands.redisValuePage(connId, key, cursor, full)),
    redisWritePrepare: (connId: string, op: RedisWriteOp_Deserialize) => unwrap(commands.redisWritePrepare(connId, op)),
    redisWriteCommit: (connId: string, pendingId: string, typedName: string | null) => unwrap(commands.redisWriteCommit(connId, pendingId, typedName)),
    redisSetMode: (dbId: string, mode: RedisWriteMode, masterPassword: string | null) => unwrap(commands.redisSetMode(dbId, mode, masterPassword)),
    redisConsoleExec: (connId: string, line: string) => unwrap(commands.redisConsoleExec(connId, line)),
    redisConsoleCommit: (connId: string, pendingId: string, typedName: string | null) => unwrap(commands.redisConsoleCommit(connId, pendingId, typedName)),
    redisConsoleHistory: (dbId: string) => unwrap(commands.redisConsoleHistory(dbId)),
    redisConsoleHistoryClear: (dbId: string) => unwrap(commands.redisConsoleHistoryClear(dbId)),
    redisCommands: (connId: string) => unwrap(commands.redisCommands(connId)),
    redisInfo: (connId: string) => unwrap(commands.redisInfo(connId)),
    redisSlowlog: (connId: string) => unwrap(commands.redisSlowlog(connId)),
    redisSubStart: (connId: string, name: RedisBytes, pattern: boolean, onEvent: (e: RedisSubEvent_Serialize) => void) => {
      const messages = new Channel<RedisSubEvent_Serialize>()
      messages.onmessage = onEvent
      return unwrap(commands.redisSubStart(connId, name, pattern, messages as never))
    },
    redisSubStop: (connId: string, name: RedisBytes, pattern: boolean) => unwrap(commands.redisSubStop(connId, name, pattern)),
    redisBulkPreview: (connId: string, pattern: string, keyType: string | null, onEvent: (e: RedisBulkEvent) => void) => {
      const progress = new Channel<RedisBulkEvent>()
      progress.onmessage = onEvent
      return unwrap(commands.redisBulkPreview(connId, pattern, keyType, progress))
    },
    redisBulkRun: (connId: string, previewId: string, action: RedisBulkAction, typedName: string | null, onEvent: (e: RedisBulkEvent) => void) => {
      const progress = new Channel<RedisBulkEvent>()
      progress.onmessage = onEvent
      return unwrap(commands.redisBulkRun(connId, previewId, action, typedName, progress))
    },
    redisBulkStop: (connId: string) => commands.redisBulkStop(connId),
    redisNodes: (connId: string) => unwrap(commands.redisNodes(connId)),
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

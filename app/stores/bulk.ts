import { defineStore } from 'pinia'
import type { BulkDone, BulkFile, BulkFileReport_Serialize as BulkFileReport, BulkImportPending, BulkProgress, CsvPreview, ExportFormat, MaskKind, MgImportMode, MgImportPending, MgImportPreview, MgImportReport } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

const message = (e: unknown, fallback: string) => (e instanceof Error ? e.message : (e as { message?: string })?.message ?? fallback)

/**
 * Bulk export and import (D17, E6) for one database tab. Rust picks the folder or zip and writes or reads every
 * file; this keeps the choices and the progress.
 */
function setup() {
  const api = useTauri()
  const chosen = ref(new Set<string>())
  const format = ref<ExportFormat>('csv')
  const header = ref(true)
  const ddl = ref(true)
  const zip = ref(false)
  const masks = reactive<{ column: string, kind: MaskKind, value: string }[]>([])
  const running = ref(false)
  const transferId = ref('')
  const progress = ref<BulkProgress | null>(null)
  const done = ref<BulkDone | null>(null)
  const error = ref('')

  async function exportMany(connId: string, objects: { schema: string, name: string }[]) {
    if (running.value || !objects.length) return
    running.value = true
    error.value = ''
    done.value = null
    progress.value = null
    transferId.value = crypto.randomUUID()
    try {
      done.value = await api.bulkExport(connId, transferId.value, {
        objects,
        format: format.value,
        header: header.value,
        masks: masks.filter(m => m.column.trim()).map(m => ({ column: m.column.trim(), kind: m.kind, value: m.value.trim() || null })),
        ddl: ddl.value,
        zip: zip.value,
      }, (p) => { progress.value = p })
    }
    catch (e) { error.value = message(e, 'The export didn\'t finish: nothing was left behind.') }
    running.value = false
    transferId.value = ''
  }
  function stop() {
    if (transferId.value) void api.sqlTransferStop(transferId.value).catch(() => {})
  }

  // ---- Bulk import (SQL): files picked from a folder or zip, each with its table, mapping or CREATE TABLE.
  interface BulkRow { file: BulkFile, include: boolean, schema: string, table: string, header: boolean, preview: CsvPreview | null, mapping: (string | null)[], ddl: string | null, cols: string[], open: boolean, error: string }
  const source = ref<{ name: string, from: string | null, skipped: string[] } | null>(null)
  const rows = ref<BulkRow[]>([])
  const pending = ref<BulkImportPending | null>(null)
  const report = ref<BulkFileReport[] | null>(null)
  async function pickImport(connId: string, isZip: boolean) {
    error.value = ''
    report.value = null
    pending.value = null
    try {
      const r = await api.bulkImportPick(connId, isZip)
      if (!r) return
      source.value = { name: r.source, from: r.from, skipped: r.skipped }
      rows.value = r.files.map(f => ({ file: f, include: true, schema: f.schema, table: f.table, header: true, preview: null, mapping: [], ddl: null, cols: [], open: false, error: '' }))
    }
    catch (e) { error.value = message(e, 'Couldn\'t read that folder.') }
  }
  /** A file going into a table that is there: D6's preview and suggested mapping. */
  async function preview(connId: string, r: BulkRow) {
    r.error = ''
    try {
      r.preview = await api.sqlImportPreview(connId, r.schema, r.table, r.file.id, r.file.delimiter, r.header)
      r.mapping = [...r.preview.suggested]
    }
    catch (e) { r.error = message(e, 'Couldn\'t read that file.') }
  }
  /** A file whose table isn't there: the CREATE TABLE to review. */
  async function infer(connId: string, r: BulkRow) {
    r.error = ''
    try {
      const t = await api.bulkInfer(connId, r.file.id, r.file.delimiter, r.header, r.schema, r.table)
      r.ddl = t.ddl
      r.cols = t.columns.map(c => c.name)
    }
    catch (e) { r.error = message(e, 'Couldn\'t read that file.') }
  }
  async function prepareImport(connId: string, exists: (r: BulkRow) => boolean) {
    error.value = ''
    report.value = null
    const chosen = rows.value.filter(r => r.include)
    // Every file needs its mapping (or its CREATE TABLE) before the review.
    for (const r of chosen) {
      if (exists(r)) {
        if (!r.preview) await preview(connId, r)
      }
      else if (r.ddl === null) {
        await infer(connId, r)
      }
    }
    if (chosen.some(r => r.error)) {
      error.value = 'Some files need a look first (see their rows).'
      return
    }
    try {
      pending.value = await api.bulkImportPrepare(connId, chosen.map(r => ({
        file_id: r.file.id,
        name: r.file.name,
        schema: r.schema,
        table: r.table,
        delimiter_name: r.file.delimiter,
        header: r.header,
        mapping: exists(r) ? r.mapping : r.cols,
        create: exists(r) ? null : r.ddl,
      })))
    }
    catch (e) { error.value = message(e, 'Couldn\'t prepare the import.') }
  }
  async function commitImport(connId: string) {
    const p = pending.value
    if (!p) return
    running.value = true
    progress.value = null
    try {
      report.value = await api.bulkImportCommit(connId, p.id, (x) => { progress.value = x })
      pending.value = null
    }
    catch (e) { error.value = message(e, 'The import didn\'t run.') }
    running.value = false
  }
  function cancelImport(connId: string) {
    const p = pending.value
    if (p) void api.bulkImportDiscard(connId, p.id).catch(() => {})
    pending.value = null
  }

  // ---- MongoDB import
  const mg = reactive({ fileId: '', name: '', preview: null as MgImportPreview | null, db: '', coll: '', mode: 'insert' as MgImportMode['mode'], field: '_id', pending: null as MgImportPending | null, typed: '', report: null as MgImportReport | null, error: '', busy: false })
  async function mgPick(connId: string) {
    mg.error = ''
    mg.report = null
    try {
      const f = await api.mgImportPick(connId)
      if (!f) return
      Object.assign(mg, { fileId: f.id, name: f.name, coll: mg.coll || f.name.replace(/\.[^.]+$/, '').split('.').pop() || '' })
      mg.preview = await api.mgImportPreview(connId, f.id)
    }
    catch (e) { mg.error = message(e, 'Couldn\'t read that file.') }
  }
  const mgMode = (): MgImportMode => (mg.mode === 'upsert' ? { mode: 'upsert', field: mg.field } : { mode: mg.mode } as MgImportMode)
  async function mgPrepare(connId: string) {
    mg.error = ''
    mg.typed = ''
    try { mg.pending = await api.mgImportPrepare(connId, mg.db, mg.coll.trim(), mg.fileId, mgMode()) }
    catch (e) { mg.error = message(e, 'Couldn\'t prepare the import.') }
  }
  async function mgCommit(connId: string) {
    const p = mg.pending
    if (!p) return
    mg.busy = true
    try {
      mg.report = await api.mgImportCommit(connId, p.id, p.level === 'typed' ? mg.typed : null, (x) => { progress.value = x })
      mg.pending = null
    }
    catch (e) { mg.error = message(e, 'The import didn\'t run.') }
    mg.busy = false
  }
  function mgCancel(connId: string) {
    const p = mg.pending
    if (p) void api.mgImportDiscard(connId, p.id).catch(() => {})
    mg.pending = null
  }

  return { chosen, format, header, ddl, zip, masks, running, progress, done, error, exportMany, stop, source, rows, pending, report, pickImport, preview, infer, prepareImport, commitImport, cancelImport, mg, mgPick, mgPrepare, mgCommit, mgCancel }
}

const makeStore = (key: string) => defineStore(`bulk:${key}`, setup)
const stores = new Map<string, ReturnType<typeof makeStore>>()

export function useBulkStore(tabKey: string) {
  if (!stores.has(tabKey)) stores.set(tabKey, makeStore(tabKey))
  return stores.get(tabKey)!()
}

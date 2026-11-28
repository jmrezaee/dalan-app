// In-app drag and drop on pointer events (P31). The window's native file drop (dragDropEnabled, for a dump dropped
// from Finder) swallows the browser's HTML5 drag events, so tabs, SFTP rows, host cards and snippets drag with
// these two directives instead:
//   v-drag="{ kind: 'tab', data: key, label: 'web-1', start, end }"   on what is dragged
//   v-drop="{ kind: 'tab', over, leave, drop }"                       on where it can land
// A drag starts after the pointer moves 5px with the button down, so clicks and double-clicks stay what they were.
// Escape cancels. The click that ends a drag isn't passed on.
import type { Directive } from 'vue'

export interface DragSpec {
  kind: string
  data: unknown
  /** Shown next to the pointer while dragging. */
  label?: string
  disabled?: boolean
  start?: () => void
  end?: (dropped: boolean) => void
}
export interface DropSpec {
  kind: string | string[]
  accepts?: (data: unknown) => boolean
  /** Every move over this target (for zones like before/after). */
  over?: (e: PointerEvent, el: HTMLElement) => void
  leave?: () => void
  drop: (data: unknown, e: PointerEvent, el: HTMLElement) => void
}

type El = HTMLElement & { __drag?: DragSpec, __drop?: DropSpec, __dragDown?: (e: PointerEvent) => void }

/** What is being dragged right now, if anything. */
export const pointerDrag = shallowRef<{ kind: string, data: unknown } | null>(null)

const THRESHOLD = 5

function targetAt(x: number, y: number, kind: string, data: unknown): El | null {
  let el = document.elementFromPoint(x, y) as El | null
  while (el) {
    const d = el.__drop
    if (d && [d.kind].flat().includes(kind) && (!d.accepts || d.accepts(data))) return el
    el = el.parentElement as El | null
  }
  return null
}

function down(e: PointerEvent, el: El) {
  const spec = el.__drag
  if (!spec || spec.disabled || e.button !== 0) return
  if ((e.target as HTMLElement).closest('input, textarea, select, [contenteditable=true]')) return
  const sx = e.clientX
  const sy = e.clientY
  let started = false
  let over: El | null = null
  let ghost: HTMLDivElement | null = null

  const move = (m: PointerEvent) => {
    if (!started) {
      if (Math.hypot(m.clientX - sx, m.clientY - sy) < THRESHOLD) return
      started = true
      pointerDrag.value = { kind: spec.kind, data: spec.data }
      spec.start?.()
      document.documentElement.dataset.dragging = spec.kind
      if (spec.label) {
        ghost = document.createElement('div')
        ghost.className = 'pointer-events-none fixed left-0 top-0 z-[100] max-w-60 truncate rounded-md border border-border bg-surface px-2 py-1 text-xs text-text shadow-drag'
        ghost.textContent = spec.label
        ghost.setAttribute('aria-hidden', 'true')
        document.body.append(ghost)
      }
    }
    m.preventDefault()
    if (ghost) ghost.style.transform = `translate(${m.clientX + 12}px, ${m.clientY + 12}px)`
    const t = targetAt(m.clientX, m.clientY, spec.kind, spec.data)
    if (t !== over) {
      over?.__drop?.leave?.()
      over = t
    }
    t?.__drop?.over?.(m, t)
  }
  const finish = (u: PointerEvent | null) => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    window.removeEventListener('pointercancel', cancel)
    window.removeEventListener('keydown', esc, true)
    if (!started) return
    ghost?.remove()
    delete document.documentElement.dataset.dragging
    over?.__drop?.leave?.()
    const t = u ? targetAt(u.clientX, u.clientY, spec.kind, spec.data) : null
    if (t && u) t.__drop!.drop(spec.data, u, t)
    pointerDrag.value = null
    spec.end?.(!!t)
    // The click the browser sends after the button comes up belongs to the drag, not to what is under it.
    const eat = (c: MouseEvent) => { c.stopPropagation(); c.preventDefault() }
    window.addEventListener('click', eat, { capture: true, once: true })
    setTimeout(() => window.removeEventListener('click', eat, { capture: true }), 0)
  }
  const up = (u: PointerEvent) => finish(u)
  const cancel = () => finish(null)
  const esc = (k: KeyboardEvent) => {
    if (k.key !== 'Escape' || !started) return
    k.preventDefault()
    k.stopPropagation()
    finish(null)
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  window.addEventListener('pointercancel', cancel)
  window.addEventListener('keydown', esc, true)
}

export const vDrag: Directive<El, DragSpec> = {
  mounted(el, b) {
    el.__drag = b.value
    el.__dragDown = e => down(e, el)
    el.addEventListener('pointerdown', el.__dragDown)
    // No native image or link drag can start under it and take the pointer away.
    el.draggable = false
  },
  updated(el, b) { el.__drag = b.value },
  unmounted(el) {
    if (el.__dragDown) el.removeEventListener('pointerdown', el.__dragDown)
    delete el.__drag
  },
}

export const vDrop: Directive<El, DropSpec> = {
  mounted(el, b) { el.__drop = b.value },
  updated(el, b) { el.__drop = b.value },
  unmounted(el) { delete el.__drop },
}

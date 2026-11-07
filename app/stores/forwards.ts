import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import { events } from '~/types/bindings'
import type { ForwardInput, ForwardSummary, PromptKind, SessionEvent } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

/** S09: port-forward rules and their live state. Rust pushes `tunnel-changed` as rules start, drop and stop. */
export const useForwardsStore = defineStore('forwards', () => {
  const api = useTauri()
  const rules = ref<ForwardSummary[]>([])
  const loaded = ref(false)
  /** A host-key or password question from a rule that is starting. */
  const prompt = ref<{ ruleId: string, id: string, kind: PromptKind } | null>(null)
  let watching = false

  function message(e: unknown) {
    return e instanceof CommandError && (e.code === 'validation' || e.code === 'in_use') ? e.message : 'Something went wrong. Try again.'
  }

  async function refresh() {
    try {
      rules.value = await api.forwardsList()
      loaded.value = true
    }
    catch {
      toast.error('Something went wrong. Try again.')
    }
  }

  /** Listens for live state once. A plain browser has no events, which is fine. */
  function watch() {
    if (watching) return
    watching = true
    events.tunnelChanged.listen((e) => {
      const r = rules.value.find(x => x.id === e.payload.id)
      if (r) Object.assign(r, { state: e.payload.state, connections: e.payload.connections, error: e.payload.error })
    }).catch(() => {})
  }

  async function save(input: ForwardInput) {
    try {
      await api.forwardSave(input)
    }
    catch (e) {
      return message(e)
    }
    await refresh()
    toast(`Saved ${input.label || 'the rule'}.`)
    return ''
  }

  async function remove(r: ForwardSummary) {
    try {
      await api.forwardDelete(r.id)
    }
    catch (e) {
      toast.error(message(e))
      return
    }
    await refresh()
    toast(`Deleted ${r.label}.`)
  }

  function onEvent(r: ForwardSummary, e: SessionEvent) {
    if (e.type === 'prompt') prompt.value = { ruleId: r.id, id: e.prompt_id, kind: e.kind }
  }

  async function start(r: ForwardSummary) {
    r.state = 'starting'
    r.error = null
    try {
      await api.forwardStart(r.id, e => onEvent(r, e))
    }
    catch (e) {
      r.state = 'error'
      r.error = message(e)
    }
  }

  function stop(r: ForwardSummary) {
    prompt.value = null
    api.forwardStop(r.id)
    r.state = 'stopped'
    r.connections = 0
    r.error = null
  }

  const toggle = (r: ForwardSummary) => (r.state === 'stopped' || r.state === 'error' ? start(r) : stop(r))

  async function answer(accept: boolean, text: string | null, remember: boolean) {
    const p = prompt.value
    if (!p) return
    prompt.value = null
    await api.forwardRespond(p.ruleId, p.id, { accept, text, remember }).catch(() => {})
  }

  function clear() {
    rules.value = []
    loaded.value = false
    prompt.value = null
  }

  return { rules, loaded, prompt, refresh, watch, save, remove, toggle, answer, clear }
})

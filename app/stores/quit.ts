import { defineStore } from 'pinia'
import { events } from '~/types/bindings'
import { useTauri } from '~/composables/useTauri'

/** Closing the window or pressing ⌘Q while sessions are live: Rust holds the quit back and sends `quit-requested`. */
export const useQuitStore = defineStore('quit', () => {
  const api = useTauri()
  /** How many live sessions the question is about, or `null` when no question is open. */
  const liveSessions = ref<number | null>(null)
  /** Scheduled jobs running now (D15): quitting stops them, and they run again at the next unlock. */
  const runningJobs = ref(0)
  let watching = false

  /** Listens once. A plain browser has no events, which is fine. */
  function watch() {
    if (watching) return
    watching = true
    events.quitRequested.listen((e) => {
      liveSessions.value = e.payload.live_sessions
      runningJobs.value = e.payload.running_jobs
    }).catch(() => {})
  }

  /** "Quit anyway": every session ends with the app. */
  function quit() {
    void api.quitApp()
  }

  /** "Keep working": Rust asks again the next time. */
  function keepWorking() {
    liveSessions.value = null
    runningJobs.value = 0
    void api.quitCancel()
  }

  return { liveSessions, runningJobs, watch, quit, keepWorking }
})

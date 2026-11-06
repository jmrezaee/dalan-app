import { defineStore } from 'pinia'
import { toast } from 'vue-sonner'
import { events } from '~/types/bindings'
import { CommandError, useTauri } from '~/composables/useTauri'

type Phase = 'loading' | 'needs-setup' | 'locked' | 'unlocked' | 'unavailable'

// Copy for error codes. brand.md wording wins.
const COPY: Record<string, string> = {
  wrong_password: 'That password didn\'t work.',
  too_many_attempts: 'Too many tries.',
  validation: 'Use at least 8 characters.',
}

export const useVaultStore = defineStore('vault', () => {
  const api = useTauri()
  const phase = ref<Phase>('loading')
  const error = ref('')
  const busy = ref(false)
  const cooldown = ref(0)
  const autoLockMinutes = ref(15)
  let timer: ReturnType<typeof setInterval> | undefined

  function startCooldown(secs: number) {
    cooldown.value = secs
    clearInterval(timer)
    timer = setInterval(() => {
      cooldown.value = Math.max(0, cooldown.value - 1)
      if (cooldown.value === 0) clearInterval(timer)
    }, 1000)
  }

  async function refresh() {
    try {
      const s = await api.vaultStatus()
      autoLockMinutes.value = s.auto_lock_minutes
      phase.value = !s.exists ? 'needs-setup' : s.unlocked ? 'unlocked' : 'locked'
      if (s.cooldown_secs) startCooldown(s.cooldown_secs)
    }
    catch {
      // Not running inside the desktop app (plain browser), or Rust failed to start.
      phase.value = 'unavailable'
    }
  }

  async function run(fn: () => Promise<unknown>, next: Phase) {
    busy.value = true
    error.value = ''
    try {
      await fn()
      phase.value = next
    }
    catch (e) {
      if (e instanceof CommandError) {
        error.value = COPY[e.code] ?? e.message
        if (e.code === 'too_many_attempts' || e.code === 'wrong_password') await refreshCooldown()
      }
      else error.value = 'Something went wrong. Try again.'
      return false
    }
    finally {
      busy.value = false
    }
    return true
  }

  async function refreshCooldown() {
    try {
      const s = await api.vaultStatus()
      if (s.cooldown_secs) startCooldown(s.cooldown_secs)
    }
    catch { /* keep the error we already have */ }
  }

  const create = (password: string) => run(() => api.vaultCreate(password), 'unlocked')
  const unlock = (password: string) => run(() => api.vaultUnlock(password), 'unlocked')
  const lock = async () => {
    await api.vaultLock()
    phase.value = 'locked'
  }

  /** Listen for auto-lock and report user activity so idle time is measured in Rust. */
  function startWatching() {
    // The .catch calls keep a plain browser (no Tauri) from throwing: events only exist in the app.
    events.vaultLocked.listen(() => {
      phase.value = 'locked'
      error.value = ''
    }).catch(() => {})
    events.autoLockWarning.listen(() => toast('Locking in 30 seconds. Move the mouse or press a key to stay unlocked.')).catch(() => {})
    let last = 0
    const ping = () => {
      const now = Date.now()
      if (phase.value === 'unlocked' && now - last > 5000) {
        last = now
        api.vaultTouch().catch(() => {})
      }
    }
    for (const ev of ['mousemove', 'keydown', 'pointerdown', 'wheel']) window.addEventListener(ev, ping, { passive: true })
  }

  return { phase, error, busy, cooldown, autoLockMinutes, refresh, create, unlock, lock, startWatching, changePassword: api.vaultChangePassword }
})

import { fail, type Handlers } from '../types'

type Keys = 'appInfo' | 'quitApp' | 'quitCancel' | 'linksTake' | 'systemIntegrationGet' | 'systemIntegrationSet' | 'openDefaultApps' | 'vaultStatus' | 'vaultCreate' | 'vaultUnlock' | 'vaultLock' | 'vaultChangePassword' | 'vaultTouch' | 'settingsGetAll' | 'settingsSet' | 'updateCheck' | 'updateInstall' | 'diagnosticsText' | 'aiGet' | 'aiSave' | 'aiSuggest' | 'historyList' | 'historyClear'

/** The password every scenario's vault opens with. A test secret, not a real one. */
export const MASTER = 'correct-horse'

export const vault: Pick<Handlers, Keys> = {
  quitApp: () => undefined,
  quitCancel: () => undefined,
  linksTake: (_a, { m }) => m.links.splice(0),
  systemIntegrationGet: (_a, { m }) => m.system,
  systemIntegrationSet: (a, { m }) => {
    if (!m.system.supported) fail('validation', 'This is a Windows setting.')
    if (typeof a.sshLinks === 'boolean') { m.system.ssh_links = a.sshLinks; if (a.sshLinks) m.system.other_handler = null }
    if (typeof a.autostart === 'boolean') m.system.autostart = a.autostart
    return m.system
  },
  openDefaultApps: () => null,
  appInfo: () => ({ version: '0.1.0', data_dir: '/home/me/.local/share/app.dalan.desktop', platform: 'linux' }),
  vaultStatus: (_a, { m }) => {
    const left = Math.ceil((m.vault.cooldownUntil - Date.now()) / 1000)
    return { exists: m.vault.exists, unlocked: m.vault.unlocked, auto_lock_minutes: m.vault.autoLockMinutes, cooldown_secs: left > 0 ? left : null }
  },
  vaultCreate: (a, { m }) => {
    if (String(a.masterPassword ?? '').length < 8) fail('validation', 'Use at least 8 characters.')
    m.vault.exists = true
    m.vault.unlocked = true
    return null
  },
  vaultUnlock: (a, { m }) => {
    // Five wrong tries start a 30 second cooldown, counted on the page clock.
    if (Date.now() < m.vault.cooldownUntil) fail('too_many_attempts', 'Too many wrong tries. Wait 30 seconds.')
    if (a.masterPassword !== MASTER) {
      m.vault.wrongTries++
      if (m.vault.wrongTries >= 5) {
        m.vault.cooldownUntil = Date.now() + 30_000
        m.vault.wrongTries = 0
      }
      fail('wrong_password', 'That password didn\'t work.')
    }
    m.vault.wrongTries = 0
    m.vault.unlocked = true
    return null
  },
  vaultLock: (_a, { m }) => {
    m.vault.unlocked = false
    return null
  },
  vaultChangePassword: (a) => {
    if (a.oldPassword !== MASTER) fail('wrong_password', 'That password didn\'t work.')
    if (String(a.newPassword ?? '').length < 8) fail('validation', 'Use at least 8 characters.')
    return null
  },
  vaultTouch: () => undefined,
  settingsGetAll: (_a, { m }) => ({ ...m.settings }),
  settingsSet: (a, { m }) => {
    if (a.key === 'ai' || a.key === 'sftp') fail('validation', 'That setting can\'t be changed from here.')
    m.settings[a.key] = a.value
    return null
  },
  updateCheck: () => null,
  updateInstall: () => null,
  diagnosticsText: (_a, { m }) => `Dalan 0.1.0 (linux)\nHosts: ${m.hosts.length}\nKeys: ${m.keys.length}\n`,
  aiGet: (_a, { m }) => ({ ...m.ai }),
  aiSave: (a, { m }) => {
    if (!/^https?:\/\//.test(String(a.input.base_url))) fail('validation', 'Enter a web address that starts with https://.')
    m.ai = { model: a.input.model, base_url: a.input.base_url, has_key: a.input.clear_key ? false : !!a.input.api_key || m.ai.has_key }
    return { ...m.ai }
  },
  aiSuggest: (a, { m }) => {
    if (!m.ai.has_key) fail('validation', 'Add your API key under Settings > AI first.')
    return { command: 'df -h', is_question: String(a.request).trim().endsWith('?') }
  },
  historyList: (a, { m }) => m.history
    .filter(h => (a.hostId === null || h.host_id === a.hostId) && (!a.query || h.command.includes(a.query)))
    .slice(0, a.limit ?? 50)
    .map(h => ({ id: h.id, command: h.command, ran_at: h.ran_at })),
  historyClear: (a, { m }) => {
    m.history = a.all ? [] : m.history.filter(h => h.host_id !== a.hostId)
    return null
  },
}

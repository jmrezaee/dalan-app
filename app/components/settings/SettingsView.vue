<script setup lang="ts">
// S23 (basics): appearance, terminal, shortcuts, security. Changes save as you make them.
import { toast } from 'vue-sonner'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { SHORTCUT_ACTIONS, defaultCombo, eventCombo, formatCombo } from '~/utils/shortcuts'

const settings = useSettingsStore()
const sections = ['Appearance', 'Terminal', 'Logs', 'AI', 'Shortcuts', 'Security', 'About'] as const
const section = ref<typeof sections[number]>('Appearance')
const changeOpen = ref(false)

// ---- AI: your own API key, used by the side panel's "Ask AI" tab. The key goes to Rust once and is never shown again.
const ai = reactive({ hasKey: false, model: '', baseUrl: '', newKey: '', loaded: false, busy: false, error: '' })
async function loadAi() {
  try {
    const c = await api.aiGet()
    Object.assign(ai, { hasKey: c.has_key, model: c.model, baseUrl: c.base_url, newKey: '', loaded: true, error: '' })
  }
  catch {
    ai.error = 'Couldn\'t load the AI settings.'
  }
}
async function saveAi(clearKey = false) {
  ai.busy = true
  ai.error = ''
  try {
    const c = await api.aiSave({ model: ai.model, base_url: ai.baseUrl, api_key: ai.newKey || null, clear_key: clearKey })
    Object.assign(ai, { hasKey: c.has_key, model: c.model, baseUrl: c.base_url, newKey: '' })
    toast(clearKey ? 'API key removed.' : 'AI settings saved.')
  }
  catch (e) {
    ai.error = e instanceof Error ? e.message : 'Couldn\'t save.'
  }
  ai.busy = false
}

// ---- About: version, updates, diagnostics.
const api = useTauri()
const info = ref<{ version: string, platform: string, data_dir: string } | null>(null)
const update = ref<{ state: 'idle' | 'checking' | 'none' | 'found' | 'installing' | 'error', version?: string, notes?: string | null, error?: string }>({ state: 'idle' })
watch(section, async (s) => {
  if (s === 'AI' && !ai.loaded) await loadAi()
  if (s === 'About' && !info.value) info.value = await api.appInfo().catch(() => null)
})
async function checkUpdate() {
  update.value = { state: 'checking' }
  try {
    const u = await api.updateCheck()
    update.value = u ? { state: 'found', version: u.version, notes: u.notes } : { state: 'none' }
  }
  catch (e) {
    update.value = { state: 'error', error: e instanceof Error ? e.message : 'Couldn\'t check for updates.' }
  }
}
async function installUpdate() {
  update.value = { ...update.value, state: 'installing' }
  try {
    await api.updateInstall() // restarts the app when it works
  }
  catch (e) {
    update.value = { state: 'error', error: e instanceof Error ? e.message : 'The update failed.' }
  }
}
async function copyDiagnostics() {
  try {
    await navigator.clipboard.writeText(await api.diagnosticsText())
    toast('Diagnostics copied. Servers, addresses and usernames are removed.')
  }
  catch {
    toast.error('Couldn\'t copy the diagnostics.')
  }
}

const THEMES = [
  { value: 'system', label: 'System', swatch: 'bg-gradient-to-r from-neutral-900 from-50% to-white to-50%' },
  { value: 'dark', label: 'Dark', swatch: 'bg-neutral-900' },
  { value: 'light', label: 'Light', swatch: 'bg-white' },
] as const
const LOCK_OPTIONS = [{ v: 0, l: 'Never' }, { v: 1, l: '1 minute' }, { v: 5, l: '5 minutes' }, { v: 15, l: '15 minutes' }, { v: 30, l: '30 minutes' }, { v: 60, l: '1 hour' }]
const fixedShortcuts = [{ name: 'Vault', keys: ['Mod+1'] }, { name: 'SFTP', keys: ['Mod+2'] }, { name: 'Switch to tab 3 to 9', keys: ['Mod+3', 'Mod+9'] }]
// ---- Shortcut recorder: press Change, then the new combination. Esc cancels.
const recording = ref<string | null>(null)
const shortcutError = ref('')
function startRecording(id: string) {
  shortcutError.value = ''
  recording.value = id
}
function onRecord(e: KeyboardEvent) {
  const id = recording.value
  if (!id) return
  e.preventDefault()
  e.stopPropagation()
  if (e.key === 'Escape') { recording.value = null; return }
  const combo = eventCombo(e)
  if (!combo) return // still holding modifiers
  const problem = settings.setShortcut(id, combo)
  if (problem) shortcutError.value = problem
  else recording.value = null
}
onMounted(() => window.addEventListener('keydown', onRecord, true))
onBeforeUnmount(() => window.removeEventListener('keydown', onRecord, true))
// Terminals need a number box that commits on change, not on every keystroke.
const num = (v: string | number, fallback: number) => (Number.isFinite(Number(v)) && v !== '' ? Number(v) : fallback)
</script>

<template>
  <PageToolbar title="Settings" />
  <div class="flex min-h-0 flex-1">
    <nav aria-label="Settings sections" class="flex w-[200px] shrink-0 flex-col gap-0.5 border-r border-border bg-surface p-2">
      <button
        v-for="s in sections"
        :key="s"
        type="button"
        :aria-current="section === s ? 'page' : undefined"
        :class="cn('relative h-8 rounded-md px-2.5 text-left text-sm text-muted hover:bg-surface-hover hover:text-text', section === s && 'bg-surface-hover text-text before:absolute before:inset-y-1 before:left-0 before:w-0.5 before:rounded-full before:bg-accent')"
        @click="section = s"
      >
        {{ s }}
      </button>
    </nav>

    <div class="min-w-0 flex-1 overflow-auto px-10 py-6">
      <div class="max-w-[680px] [&>.row]:flex [&>.row]:items-center [&>.row]:gap-4 [&>.row]:border-b [&>.row]:border-border [&>.row]:py-3.5">
        <h1 class="mb-2 text-xl">{{ section }}</h1>

        <template v-if="section === 'Appearance'">
          <div class="row">
            <span class="flex flex-1 flex-col">Theme<span class="text-xs text-muted">Follows your system unless you pick one.</span></span>
            <div role="radiogroup" aria-label="Theme" class="flex gap-2">
              <button
                v-for="t in THEMES"
                :key="t.value"
                type="button"
                role="radio"
                :aria-checked="settings.theme === t.value"
                :class="cn('w-[88px] rounded-lg border border-border p-1.5 text-center text-xs', settings.theme === t.value && 'border-accent ring-1 ring-accent')"
                @click="settings.setTheme(t.value)"
              >
                <span :class="cn('mb-1 block h-10 rounded border border-border', t.swatch)" />{{ t.label }}
              </button>
            </div>
          </div>
        </template>

        <template v-else-if="section === 'Terminal'">
          <div class="row">
            <span class="flex-1">Font size</span>
            <div class="w-24"><NumberInput :model-value="settings.terminal.font_size" :min="8" :max="32" @update:model-value="(v: string | number) => settings.setTerminal({ font_size: num(v, 14) })" /></div>
          </div>
          <div class="row">
            <span class="flex-1">Cursor</span>
            <div class="w-64"><Segmented :model-value="settings.terminal.cursor" label="Cursor" :options="[{ value: 'block', label: 'Block' }, { value: 'bar', label: 'Bar' }, { value: 'underline', label: 'Underline' }]" @update:model-value="(v: string) => settings.setTerminal({ cursor: v as 'block' | 'bar' | 'underline' })" /></div>
          </div>
          <div class="row"><span class="flex-1">Blinking cursor</span><Switch :model-value="settings.terminal.cursor_blink" @update:model-value="(v: boolean) => settings.setTerminal({ cursor_blink: v })" /></div>
          <div class="row">
            <span class="flex-1">Scrollback lines</span>
            <div class="w-32"><NumberInput :model-value="settings.terminal.scrollback" :min="500" :max="100000" @update:model-value="(v: string | number) => settings.setTerminal({ scrollback: num(v, 10000) })" /></div>
          </div>
          <div class="row"><span class="flex flex-1 flex-col">Copy on select<span class="text-xs text-muted">Copies whatever you highlight in a terminal.</span></span><Switch :model-value="settings.terminal.copy_on_select" @update:model-value="(v: boolean) => settings.setTerminal({ copy_on_select: v })" /></div>
          <div class="row"><span class="flex flex-1 flex-col">Detect server OS<span class="text-xs text-muted">On the first connect to a server, runs <span class="font-mono">uname</span> once to show its OS on the host card. Nothing else is read.</span></span><Switch :model-value="settings.detectOs" aria-label="Detect server OS" @update:model-value="(v: boolean) => settings.setDetectOs(v)" /></div>
          <p class="pt-3 text-xs text-muted">Changes apply to open terminals straight away.</p>
        </template>

        <template v-else-if="section === 'Logs'">
          <div class="row">
            <span class="flex flex-1 flex-col">Keep a log of each session<span class="text-xs text-muted">Saves what the terminal printed, encrypted with your master password. Each server keeps its latest log; bookmark one to keep it. Applies to sessions you open from now on.</span></span>
            <Switch :model-value="settings.logsEnabled" aria-label="Keep session logs" @update:model-value="(v: boolean) => settings.setLogs(v)" />
          </div>
          <div class="row">
            <span class="flex flex-1 flex-col">Remember typed commands<span class="text-xs text-muted">Fills the History tab in a terminal's side panel (⌘.), per server. Lines you edit with arrows or Tab, and lines typed at a password prompt, are skipped.</span></span>
            <Switch :model-value="settings.historyEnabled" aria-label="Remember typed commands" @update:model-value="(v: boolean) => settings.setHistory(v)" />
          </div>
          <p class="pt-3 text-xs text-muted">Commands and logs are encrypted on this computer. Lines that carry an obvious secret (--password, TOKEN=, mysql -pSECRET) are not saved, but that is a guess, so a secret in an unusual spot can still be kept. Logs can hold anything shown on screen, so turn this on only if that's fine for your servers. Old logs stay readable after you change the master password.</p>
        </template>

        <template v-else-if="section === 'Shortcuts'">
          <div v-for="a in SHORTCUT_ACTIONS" :key="a.id" class="row">
            <span class="flex-1">{{ a.label }}</span>
            <span v-if="recording === a.id" class="text-sm text-accent" role="status">Press the new shortcut…</span>
            <Kbd v-else>{{ formatCombo(settings.combos[a.id]!) }}</Kbd>
            <Button variant="secondary" size="sm" @click="recording === a.id ? (recording = null) : startRecording(a.id)">{{ recording === a.id ? 'Cancel' : 'Change' }}</Button>
            <Button v-if="settings.combos[a.id] !== defaultCombo(a)" variant="ghost" size="sm" :aria-label="`Reset ${a.label}`" @click="settings.setShortcut(a.id, null)">Reset</Button>
          </div>
          <div v-for="f in fixedShortcuts" :key="f.name" class="row text-muted">
            <span class="flex-1">{{ f.name }}</span><Kbd>{{ f.keys.map(k => formatCombo(k)).join(' to ') }}</Kbd><span class="w-[66px]" />
          </div>
          <p v-if="shortcutError" role="alert" class="pt-3 text-sm text-danger">{{ shortcutError }}</p>
          <p class="pt-3 text-xs text-muted">Shortcuts use the {{ formatCombo('Mod+X').startsWith('⌘') ? '⌘' : 'Ctrl' }} key. Press Change, then the new combination.</p>
        </template>

        <template v-else-if="section === 'AI'">
          <p class="py-3 text-sm text-muted">The side panel's <strong class="font-medium text-text">Ask AI</strong> tab turns a request like "find files over 1 GB" into a shell command using your own Anthropic API key. It only types the command into the terminal for you to check and run.</p>
          <div class="row">
            <span class="flex flex-1 flex-col">API key<span class="text-xs text-muted">{{ ai.hasKey ? 'A key is saved, encrypted with your master password. Paste a new one to replace it.' : 'Stored encrypted with your master password. It never leaves this computer except in requests to the service below.' }}</span></span>
            <div class="w-64"><PasswordInput v-model="ai.newKey" aria-label="API key" :placeholder="ai.hasKey ? '••••••••' : 'sk-ant-…'" autocomplete="off" /></div>
          </div>
          <div class="row">
            <span class="flex-1">Model</span>
            <input v-model="ai.model" aria-label="Model" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" class="h-9 w-64 rounded-md border border-border-input bg-bg px-3 font-mono text-xs outline-none focus:border-accent">
          </div>
          <div class="row">
            <span class="flex flex-1 flex-col">Service address<span class="text-xs text-muted">Leave as is unless you run your own gateway. Must be https.</span></span>
            <input v-model="ai.baseUrl" aria-label="Service address" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" class="h-9 w-64 rounded-md border border-border-input bg-bg px-3 font-mono text-xs outline-none focus:border-accent">
          </div>
          <p v-if="ai.error" role="alert" class="pt-3 text-sm text-danger">{{ ai.error }}</p>
          <div class="flex gap-2 pt-4">
            <Button :loading="ai.busy" @click="saveAi(false)">Save</Button>
            <Button v-if="ai.hasKey" variant="secondary" :disabled="ai.busy" @click="saveAi(true)">Remove key</Button>
          </div>
          <p class="pt-4 text-xs text-muted">What is sent: your request text and the server's OS name. Not sent: terminal output, host names, files or other servers. Usage is billed by your provider.</p>
        </template>

        <template v-else-if="section === 'About'">
          <div class="row"><span class="flex-1">Version</span><span class="font-mono text-sm">{{ info?.version ?? '…' }}</span></div>
          <div class="row"><span class="flex-1">System</span><span class="text-sm text-muted">{{ info?.platform ?? '…' }}</span></div>
          <div class="row"><span class="flex-1">Data folder</span><span class="truncate font-mono text-xs text-muted" :title="info?.data_dir">{{ info?.data_dir ?? '…' }}</span></div>
          <div class="row">
            <span class="flex flex-1 flex-col">Updates
              <span v-if="update.state === 'none'" class="text-xs text-muted">You're on the newest version.</span>
              <span v-else-if="update.state === 'found'" class="text-xs text-muted">Version {{ update.version }} is available.</span>
              <span v-else-if="update.state === 'installing'" class="text-xs text-muted">Downloading and installing. Dalan restarts when it's done.</span>
              <span v-else-if="update.state === 'error'" role="alert" class="text-xs text-danger">{{ update.error }}</span>
              <span v-else class="text-xs text-muted">Checks GitHub Releases. Nothing is sent but the request itself.</span>
            </span>
            <Button v-if="update.state === 'found'" @click="installUpdate">Install and restart</Button>
            <Button v-else variant="secondary" :loading="update.state === 'checking' || update.state === 'installing'" @click="checkUpdate">Check for updates</Button>
          </div>
          <div class="row">
            <span class="flex flex-1 flex-col">Copy diagnostics<span class="text-xs text-muted">Version, system and the last 200 log lines, with servers, addresses and usernames removed. Paste it into an issue.</span></span>
            <Button variant="secondary" @click="copyDiagnostics">Copy</Button>
          </div>
        </template>

        <template v-else>
          <div class="row">
            <span class="flex-1">Lock after idle</span>
            <Select :model-value="String(settings.autoLockMinutes)" @update:model-value="(v) => settings.setAutoLock(Number(v))">
              <SelectTrigger class="w-40"><SelectValue /></SelectTrigger>
              <SelectContent><SelectItem v-for="o in LOCK_OPTIONS" :key="o.v" :value="String(o.v)">{{ o.l }}</SelectItem></SelectContent>
            </Select>
          </div>
          <div class="row">
            <span class="flex flex-1 flex-col">Change master password<span class="text-xs text-muted">Re-encrypts every secret in one step.</span></span>
            <Button variant="secondary" @click="changeOpen = true">Change…</Button>
          </div>
        </template>
      </div>
    </div>
  </div>
  <ChangePasswordDialog v-model:open="changeOpen" />
</template>

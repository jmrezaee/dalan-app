<script setup lang="ts">
// S23 (basics): appearance, terminal, shortcuts, security. Changes save as you make them.
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'

const settings = useSettingsStore()
const sections = ['Appearance', 'Terminal', 'Shortcuts', 'Security'] as const
const section = ref<typeof sections[number]>('Appearance')
const changeOpen = ref(false)

const THEMES = [
  { value: 'system', label: 'System', swatch: 'bg-gradient-to-r from-neutral-900 from-50% to-white to-50%' },
  { value: 'dark', label: 'Dark', swatch: 'bg-neutral-900' },
  { value: 'light', label: 'Light', swatch: 'bg-white' },
] as const
const LOCK_OPTIONS = [{ v: 0, l: 'Never' }, { v: 1, l: '1 minute' }, { v: 5, l: '5 minutes' }, { v: 15, l: '15 minutes' }, { v: 30, l: '30 minutes' }, { v: 60, l: '1 hour' }]
const SHORTCUTS = [
  ['Command palette', '⌘K'], ['New tab', '⌘T'], ['Close tab', '⌘W'], ['New host', '⌘N'],
  ['Hosts', '⌘1'], ['Switch to tab 1 to 8', '⌘2 to ⌘9'], ['Settings', '⌘,'],
]
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
          <p class="pt-3 text-xs text-muted">Changes apply to open terminals straight away.</p>
        </template>

        <template v-else-if="section === 'Shortcuts'">
          <div v-for="[name, keys] in SHORTCUTS" :key="name" class="row">
            <span class="flex-1">{{ name }}</span><Kbd>{{ keys }}</Kbd>
          </div>
          <p class="pt-3 text-xs text-muted">Rebinding shortcuts comes later.</p>
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

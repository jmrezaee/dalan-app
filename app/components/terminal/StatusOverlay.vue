<script setup lang="ts">
// S18. Copy is from replica/brand.md. The overlay is role=alertdialog when it needs an answer.
import type { AuthMethod } from '~/types/bindings'
import { CircleAlert, KeyRound, LoaderCircle, PlugZap, ShieldAlert, Unplug } from '@lucide/vue'

export type OverlayKind = 'connecting' | 'password' | 'passphrase' | 'question' | 'host-key-new' | 'host-key-changed' | 'auth-failed' | 'unreachable' | 'disconnected'
const props = defineProps<{
  kind: OverlayKind
  host: string
  address?: string
  hop?: string
  fingerprint?: string
  oldFingerprint?: string
  reason?: string
  user?: string
  keyLabel?: string
  /** One keyboard-interactive question from the server (2FA code, PAM challenge). */
  question?: { name: string, instructions: string, prompt: string, echo: boolean }
  time?: string
  /** Plain-language progress of this connection, for "Show details". */
  steps?: string[]
  /** Ways the server accepts, offered after a sign-in failure. */
  methods?: AuthMethod[]
}>()
const emit = defineEmits<{ cancel: []; submit: [password: string, remember: boolean]; trust: []; trustOnce: []; close: []; replace: []; edit: []; retry: []; reconnect: []; tryMethod: [method: AuthMethod] }>()
const methodLabel: Record<AuthMethod, string> = { password: 'Password', key: 'Saved key', keyboard_interactive: 'Verification code' }
const needsAnswer = computed(() => props.kind !== 'connecting')
const password = ref('')
const remember = ref(false)
// The password field is inside a PasswordInput wrapper, so focus it by id when the card appears.
function focusPassword() {
  if (props.kind === 'password' || props.kind === 'passphrase') nextTick(() => document.getElementById('session-password')?.focus())
  if (props.kind === 'question') nextTick(() => document.getElementById('session-answer')?.focus())
}
// Keyboard: the safe default button (Close, Reconnect, Retry, Try again) takes focus when the card appears,
// so Enter does what the card's default says. (The `autofocus` attribute does nothing on elements added later.)
const card = ref<HTMLElement | null>(null)
function focusDefault() {
  focusPassword()
  nextTick(() => card.value?.querySelector<HTMLElement>('[data-autofocus]')?.focus())
}
// Escape picks the safe answer: refuse, or close the blocked connection. Error cards have no destructive default.
function onEscape() {
  if (['connecting', 'password', 'passphrase', 'question', 'host-key-new'].includes(props.kind)) emit('cancel')
  else if (props.kind === 'host-key-changed') emit('close')
}
onMounted(focusDefault)
watch(() => props.kind, focusDefault)
function submit() {
  emit('submit', password.value, remember.value)
  password.value = ''
}
</script>

<template>
  <!-- z-20: xterm stacks its own canvases (z-index up to 5) over anything unpositioned, which swallowed clicks. -->
  <div class="absolute inset-0 z-20 grid place-items-center bg-bg/80 backdrop-blur-[2px]">
    <div
      ref="card"
      :role="needsAnswer ? 'alertdialog' : 'status'"
      class="flex w-[420px] flex-col gap-3 rounded-lg border border-border bg-surface-raised p-5 shadow-pop"
      @keydown.esc.stop.prevent="onEscape"
    >
      <template v-if="kind === 'connecting'">
        <LoaderCircle class="size-6 animate-spin text-accent" aria-hidden="true" />
        <h2 class="text-lg">Connecting to {{ host }}…</h2>
        <p v-if="hop" class="font-mono text-xs text-muted">via {{ hop }}</p>
        <ConnectionDetails :steps="steps ?? []" />
        <div class="flex justify-end"><Button variant="secondary" @click="$emit('cancel')">Cancel</Button></div>
      </template>

      <form v-else-if="kind === 'password' || kind === 'passphrase'" class="flex flex-col gap-3" @submit.prevent="submit">
        <KeyRound class="size-6 text-accent" aria-hidden="true" />
        <h2 class="text-lg">{{ kind === 'passphrase' ? `Passphrase for ${keyLabel}` : `Password for ${host}` }}</h2>
        <p v-if="kind === 'passphrase'" class="text-muted">This key is protected. Dalan opens it to sign in to {{ host }}.</p>
        <p v-else-if="user" class="text-muted">Signing in as <span class="font-mono">{{ user }}</span>.</p>
        <PasswordInput id="session-password" v-model="password" :label="kind === 'passphrase' ? 'Key passphrase' : 'Password'" />
        <label class="flex items-center gap-2 text-sm"><Checkbox v-model="remember" />Remember it on this computer</label>
        <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="$emit('cancel')">Cancel</Button><Button type="submit">Continue</Button></div>
      </form>

      <form v-else-if="kind === 'question' && question" class="flex flex-col gap-3" @submit.prevent="submit">
        <KeyRound class="size-6 text-accent" aria-hidden="true" />
        <h2 class="text-lg">{{ question.name || `Sign in to ${host}` }}</h2>
        <p v-if="question.instructions" class="whitespace-pre-line text-muted">{{ question.instructions }}</p>
        <label for="session-answer" class="text-sm">{{ question.prompt.trim() }}</label>
        <Input v-if="question.echo" id="session-answer" v-model="password" autocomplete="off" />
        <PasswordInput v-else id="session-answer" v-model="password" />
        <div class="flex justify-end gap-2"><Button type="button" variant="secondary" @click="$emit('cancel')">Cancel</Button><Button type="submit">Continue</Button></div>
      </form>

      <template v-else-if="kind === 'host-key-new'">
        <ShieldAlert class="size-6 text-warning" aria-hidden="true" />
        <h2 class="text-lg">First time connecting to {{ host }}</h2>
        <p class="text-muted">Check that this fingerprint matches the server:</p>
        <code class="break-all rounded-md bg-bg px-2 py-1.5 font-mono text-xs">{{ fingerprint }}</code>
        <div class="flex justify-end gap-2"><Button variant="secondary" data-autofocus @click="$emit('cancel')">Cancel</Button><Button variant="secondary" @click="$emit('trustOnce')">Connect once</Button><Button @click="$emit('trust')">Trust and connect</Button></div>
        <p class="text-xs text-muted">"Connect once" signs in now and saves nothing, so you are asked again next time.</p>
      </template>

      <template v-else-if="kind === 'host-key-changed'">
        <ShieldAlert class="size-6 text-danger" aria-hidden="true" />
        <h2 class="text-lg">This server's identity changed</h2>
        <p class="text-muted">That can happen after a reinstall, or it can mean someone is intercepting the connection. Don't continue unless you expected it.</p>
        <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-xs">
          <dt class="text-muted">Saved</dt><dd class="break-all">{{ oldFingerprint }}</dd>
          <dt class="text-muted">Now</dt><dd class="break-all text-danger">{{ fingerprint }}</dd>
        </dl>
        <div class="flex items-center justify-between">
          <Button variant="link" size="sm" @click="$emit('replace')">Replace saved key…</Button>
          <Button data-autofocus @click="$emit('close')">Close</Button>
        </div>
      </template>

      <template v-else-if="kind === 'auth-failed'">
        <CircleAlert class="size-6 text-danger" aria-hidden="true" />
        <h2 class="text-lg">Sign-in failed</h2>
        <p class="text-muted">{{ reason }}</p>
        <ConnectionDetails :steps="steps ?? []" open />
        <div v-if="methods?.length" class="flex flex-wrap items-center gap-2" role="group" aria-label="Try another way">
          <span class="text-sm text-muted">Try another way:</span>
          <Button v-for="m in methods" :key="m" variant="secondary" size="sm" @click="$emit('tryMethod', m)">{{ methodLabel[m] }}</Button>
        </div>
        <div class="flex justify-end gap-2"><Button variant="secondary" @click="$emit('edit')">Edit host</Button><Button data-autofocus @click="$emit('retry')">Try again</Button></div>
      </template>

      <template v-else-if="kind === 'unreachable'">
        <PlugZap class="size-6 text-danger" aria-hidden="true" />
        <h2 class="text-lg">Can't reach {{ address }}</h2>
        <p class="text-muted">Check the address and port, your VPN, or a firewall in between.</p>
        <ConnectionDetails :steps="steps ?? []" open />
        <div class="flex justify-end"><Button data-autofocus @click="$emit('retry')">Retry</Button></div>
      </template>

      <template v-else>
        <Unplug class="size-6 text-muted" aria-hidden="true" />
        <h2 class="text-lg">Connection closed</h2>
        <p class="text-muted">{{ host }} disconnected at {{ time }}.</p>
        <div class="flex justify-end gap-2"><Button variant="secondary" @click="$emit('close')">Close tab</Button><Button data-autofocus @click="$emit('reconnect')">Reconnect</Button></div>
      </template>
    </div>
  </div>
</template>

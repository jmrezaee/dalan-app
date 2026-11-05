<script setup lang="ts">
// A question a connection needs answered (host key, password, key passphrase), drawn inline.
// Used where there is no terminal overlay: installing a key (S08) and starting a tunnel (S09).
import { ShieldAlert } from '@lucide/vue'
import { Checkbox } from '@/components/ui/checkbox'
import type { PromptKind } from '~/types/bindings'

const props = defineProps<{ kind: PromptKind, host: string }>()
const emit = defineEmits<{ reply: [accept: boolean, text: string | null, remember: boolean] }>()

const text = ref('')
const remember = ref(false)
const secret = computed(() => props.kind.type === 'password' || props.kind.type === 'passphrase')
function reply(accept: boolean) {
  emit('reply', accept, text.value || null, remember.value)
  text.value = ''
}
</script>

<template>
  <div class="flex flex-col gap-2 text-xs">
    <template v-if="kind.type === 'host_key_new'">
      <span class="flex items-center gap-2 text-sm text-warning"><ShieldAlert class="size-4" />First time connecting to {{ kind.host || host }}</span>
      <span class="text-muted">Check that this fingerprint matches the server:</span>
      <code class="break-all rounded bg-bg px-2 py-1 font-mono">{{ kind.fingerprint }}</code>
      <div class="flex justify-end gap-2"><Button type="button" size="sm" variant="secondary" @click="reply(false)">Cancel</Button><Button type="button" size="sm" @click="reply(true)">Trust and connect</Button></div>
    </template>
    <template v-else-if="kind.type === 'host_key_changed'">
      <span class="flex items-center gap-2 text-sm text-danger"><ShieldAlert class="size-4" />This server's identity changed</span>
      <span class="text-muted">That can happen after a reinstall, or it can mean someone is intercepting the connection. Open the host in a tab to review it.</span>
      <div class="flex justify-end"><Button type="button" size="sm" @click="reply(false)">Close</Button></div>
    </template>
    <template v-else-if="kind.type === 'keyboard_interactive'">
      <span class="text-sm">{{ kind.name || `Sign in to ${kind.host}` }}</span>
      <span v-if="kind.instructions" class="whitespace-pre-line text-muted">{{ kind.instructions }}</span>
      <label for="prompt-answer">{{ kind.prompt.trim() }}</label>
      <Input v-if="kind.echo" id="prompt-answer" v-model="text" autocomplete="off" />
      <PasswordInput v-else id="prompt-answer" v-model="text" />
      <div class="flex justify-end gap-2"><Button type="button" size="sm" variant="secondary" @click="reply(false)">Cancel</Button><Button type="button" size="sm" @click="reply(true)">Continue</Button></div>
    </template>
    <template v-else-if="secret">
      <span class="text-sm">{{ kind.type === 'passphrase' ? `Passphrase for ${kind.key_label}` : `Password for ${kind.host}` }}</span>
      <PasswordInput id="prompt-secret" v-model="text" />
      <label class="flex items-center gap-2"><Checkbox v-model="remember" />Remember it on this computer</label>
      <div class="flex justify-end gap-2"><Button type="button" size="sm" variant="secondary" @click="reply(false)">Cancel</Button><Button type="button" size="sm" @click="reply(true)">Continue</Button></div>
    </template>
  </div>
</template>

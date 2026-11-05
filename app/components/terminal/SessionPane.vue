<script setup lang="ts">
// S18: the terminal plus whatever the connection needs from the user, as an overlay card.
import type { Session } from '~/stores/sessions'
import type { OverlayKind } from './StatusOverlay.vue'
import { Input } from '@/components/ui/input'
import { AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'

const props = defineProps<{ session: Session, active: boolean }>()
defineEmits<{ editHost: [hostId: string] }>()
const sessions = useSessionsStore()

const overlay = computed<OverlayKind | null>(() => {
  const s = props.session
  if (s.prompt?.kind.type === 'host_key_new') return 'host-key-new'
  if (s.prompt?.kind.type === 'host_key_changed') return 'host-key-changed'
  if (s.prompt?.kind.type === 'password') return 'password'
  if (s.prompt?.kind.type === 'passphrase') return 'passphrase'
  if (s.status === 'error') return s.error?.code === 'host_unreachable' ? 'unreachable' : 'auth-failed'
  if (s.status === 'disconnected') return 'disconnected'
  if (s.status === 'connecting' || s.status === 'authenticating') return 'connecting'
  return null
})

const kind = computed(() => props.session.prompt?.kind)
const fingerprint = computed(() => {
  const k = kind.value
  return k?.type === 'host_key_new' ? k.fingerprint : k?.type === 'host_key_changed' ? k.new_fingerprint : undefined
})
const oldFingerprint = computed(() => (kind.value?.type === 'host_key_changed' ? kind.value.old_fingerprint : undefined))
const user = computed(() => (kind.value?.type === 'password' ? kind.value.user : undefined))
const keyLabel = computed(() => (kind.value?.type === 'passphrase' ? kind.value.key_label : undefined))
const time = computed(() => props.session.disconnectedAt?.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))
const reason = computed(() => props.session.error?.message ?? '')

// "Replace saved key…" needs the host name typed first (F12).
const replaceOpen = ref(false)
const typed = ref('')
const canReplace = computed(() => typed.value.trim() === props.session.label)
function confirmReplace() {
  replaceOpen.value = false
  typed.value = ''
  sessions.respond(props.session, true)
}
// Cancel on any prompt answers "no"; closing the tab while a prompt is open ends the session.
const cancel = () => sessions.respond(props.session, false)
</script>

<template>
  <div class="relative size-full">
    <TerminalView :session="session" :active="active" />
    <StatusOverlay
      v-if="overlay"
      :kind="overlay"
      :host="session.label"
      :address="session.address"
      :fingerprint="fingerprint"
      :old-fingerprint="oldFingerprint"
      :user="user"
      :key-label="keyLabel"
      :reason="reason"
      :time="time"
      @cancel="session.prompt ? cancel() : sessions.close(session)"
      @submit="(pw, remember) => sessions.respond(session, true, pw, remember)"
      @trust="sessions.respond(session, true)"
      @close="session.prompt ? cancel() : sessions.close(session)"
      @replace="replaceOpen = true"
      @edit="$emit('editHost', session.hostId)"
      @retry="sessions.reconnect(session)"
      @reconnect="sessions.reconnect(session)"
    />

    <AlertDialog v-model:open="replaceOpen">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Replace the saved key for {{ session.label }}?</AlertDialogTitle>
          <AlertDialogDescription>Only do this if you know the server was reinstalled or its key was rotated. Type the host name to confirm.</AlertDialogDescription>
        </AlertDialogHeader>
        <Input v-model="typed" :placeholder="session.label" aria-label="Host name" autocomplete="off" @keydown.enter.prevent="canReplace && confirmReplace()" />
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button variant="destructive" :disabled="!canReplace" @click="confirmReplace">Replace and connect</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>

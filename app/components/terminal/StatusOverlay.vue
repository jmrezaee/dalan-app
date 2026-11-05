<script setup lang="ts">
// S18. Copy is from replica/brand.md. The overlay is role=alertdialog when it needs an answer.
import { CircleAlert, LoaderCircle, PlugZap, ShieldAlert, Unplug } from '@lucide/vue'

export type OverlayKind = 'connecting' | 'host-key-new' | 'host-key-changed' | 'auth-failed' | 'unreachable' | 'disconnected'
const props = defineProps<{
  kind: OverlayKind
  host: string
  address?: string
  hop?: string
  fingerprint?: string
  oldFingerprint?: string
  reason?: string
  time?: string
}>()
defineEmits<{ cancel: []; trust: []; close: []; replace: []; edit: []; retry: []; reconnect: [] }>()
const needsAnswer = computed(() => props.kind !== 'connecting')
</script>

<template>
  <div class="absolute inset-0 grid place-items-center bg-bg/80 backdrop-blur-[2px]">
    <div
      :role="needsAnswer ? 'alertdialog' : 'status'"
      class="flex w-[420px] flex-col gap-3 rounded-lg border border-border bg-surface-raised p-5 shadow-pop"
    >
      <template v-if="kind === 'connecting'">
        <LoaderCircle class="size-6 animate-spin text-accent" aria-hidden="true" />
        <h2 class="text-lg">Connecting to {{ host }}…</h2>
        <p v-if="hop" class="font-mono text-xs text-muted">via {{ hop }}</p>
        <div class="flex justify-end"><Button variant="secondary" @click="$emit('cancel')">Cancel</Button></div>
      </template>

      <template v-else-if="kind === 'host-key-new'">
        <ShieldAlert class="size-6 text-warning" aria-hidden="true" />
        <h2 class="text-lg">First time connecting to {{ host }}</h2>
        <p class="text-muted">Check that this fingerprint matches the server:</p>
        <code class="break-all rounded-md bg-bg px-2 py-1.5 font-mono text-xs">{{ fingerprint }}</code>
        <div class="flex justify-end gap-2"><Button variant="secondary" @click="$emit('cancel')">Cancel</Button><Button @click="$emit('trust')">Trust and connect</Button></div>
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
          <Button autofocus @click="$emit('close')">Close</Button>
        </div>
      </template>

      <template v-else-if="kind === 'auth-failed'">
        <CircleAlert class="size-6 text-danger" aria-hidden="true" />
        <h2 class="text-lg">Sign-in failed</h2>
        <p class="text-muted">{{ reason }}</p>
        <div class="flex justify-end gap-2"><Button variant="secondary" @click="$emit('edit')">Edit host</Button><Button @click="$emit('retry')">Try again</Button></div>
      </template>

      <template v-else-if="kind === 'unreachable'">
        <PlugZap class="size-6 text-danger" aria-hidden="true" />
        <h2 class="text-lg">Can't reach {{ address }}</h2>
        <p class="text-muted">Check the address and port, your VPN, or a firewall in between.</p>
        <div class="flex justify-end"><Button @click="$emit('retry')">Retry</Button></div>
      </template>

      <template v-else>
        <Unplug class="size-6 text-muted" aria-hidden="true" />
        <h2 class="text-lg">Connection closed</h2>
        <p class="text-muted">{{ host }} disconnected at {{ time }}.</p>
        <div class="flex justify-end gap-2"><Button variant="secondary" @click="$emit('close')">Close tab</Button><Button autofocus @click="$emit('reconnect')">Reconnect</Button></div>
      </template>
    </div>
  </div>
</template>

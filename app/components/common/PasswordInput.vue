<script setup lang="ts">
// Secrets never come back from Rust, so a saved secret shows dots and a "Change" link instead of a value.
import { Eye, EyeOff } from '@lucide/vue'
import { Input } from '@/components/ui/input'
import IconButton from './IconButton.vue'

const props = defineProps<{ saved?: boolean; invalid?: boolean; id?: string; disabled?: boolean; label?: string }>()
const model = defineModel<string>({ default: '' })
const shown = ref(false)
const changing = ref(false)
const showSaved = computed(() => props.saved && !changing.value)
</script>

<template>
  <div v-if="showSaved" class="flex items-center gap-2">
    <Input :id="id" value="••••••••" readonly aria-label="Saved password" />
    <Button variant="link" size="sm" @click="changing = true">Change</Button>
  </div>
  <div v-else class="relative">
    <Input
      :id="id"
      v-model="model"
      :type="shown ? 'text' : 'password'"
      :aria-invalid="invalid || undefined"
      :aria-label="label"
      :disabled="disabled"
      autocomplete="off"
      class="pr-9"
    />
    <IconButton
      :label="shown ? 'Hide password' : 'Show password'"
      size="xs"
      :pressed="shown"
      class="absolute right-1 top-1"
      @click="shown = !shown"
    >
      <EyeOff v-if="shown" /><Eye v-else />
    </IconButton>
  </div>
</template>

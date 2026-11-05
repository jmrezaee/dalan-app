<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { useVModel } from "@vueuse/core"
import { cn } from "@/lib/utils"

const props = defineProps<{
  defaultValue?: string | number
  modelValue?: string | number
  class?: HTMLAttributes["class"]
}>()

const emits = defineEmits<{
  (e: "update:modelValue", payload: string | number): void
}>()

const modelValue = useVModel(props, "modelValue", emits, {
  passive: true,
  defaultValue: props.defaultValue,
})
</script>

<template>
  <input
    v-model="modelValue"
    data-slot="input"
    autocapitalize="off"
    autocorrect="off"
    spellcheck="false"
    :class="cn(
      'h-8 w-full min-w-0 rounded-md border border-border-input bg-bg px-2.5 text-sm text-text outline-none transition-colors duration-100 placeholder:text-muted hover:border-muted disabled:cursor-not-allowed disabled:bg-surface disabled:opacity-60 read-only:bg-surface',
      'focus-visible:border-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/35',
      'aria-invalid:border-danger aria-invalid:ring-danger/25',
      props.class,
    )"
  >
</template>

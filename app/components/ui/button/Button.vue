<script setup lang="ts">
import type { PrimitiveProps } from "reka-ui"
import type { HTMLAttributes } from "vue"
import type { ButtonVariants } from "."
import { LoaderCircle } from "@lucide/vue"
import { Primitive } from "reka-ui"
import { cn } from "@/lib/utils"
import { buttonVariants } from "."

interface Props extends PrimitiveProps {
  variant?: ButtonVariants["variant"]
  size?: ButtonVariants["size"]
  class?: HTMLAttributes["class"]
  /** Spinner replaces the leading icon, the label stays, the button is not clickable. */
  loading?: boolean
}

const props = withDefaults(defineProps<Props>(), { as: "button" })
</script>

<template>
  <Primitive
    data-slot="button"
    :data-variant="variant"
    :data-size="size"
    :as="as"
    :as-child="asChild"
    :aria-busy="loading || undefined"
    :disabled="as === 'button' ? (loading || undefined) : undefined"
    :class="cn(buttonVariants({ variant, size }), props.class)"
  >
    <LoaderCircle v-if="loading" class="animate-spin" aria-hidden="true" />
    <slot />
  </Primitive>
</template>

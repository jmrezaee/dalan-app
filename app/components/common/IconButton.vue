<script setup lang="ts">
// Icon-only button. `label` is required: it is the aria-label and the tooltip.
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

// Tooltip renders no element of its own, so class/style/attrs must go to the Button explicitly.
defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<{
  label: string
  size?: 'xs' | 'sm'
  pressed?: boolean
  disabled?: boolean
  shortcut?: string
}>(), { size: 'sm', shortcut: undefined })
</script>

<template>
  <Tooltip>
    <TooltipTrigger as-child>
      <Button
        variant="ghost"
        :size="props.size === 'xs' ? 'icon-xs' : 'icon-sm'"
        :aria-label="label"
        :aria-pressed="pressed"
        :disabled="disabled"
        v-bind="$attrs"
        :class="cn(pressed && 'bg-surface-hover text-accent', $attrs.class as string)"
      >
        <slot />
      </Button>
    </TooltipTrigger>
    <TooltipContent>
      {{ label }}
      <Kbd v-if="shortcut" class="ml-2">{{ shortcut }}</Kbd>
    </TooltipContent>
  </Tooltip>
</template>

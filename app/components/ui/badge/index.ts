import type { VariantProps } from "class-variance-authority"
import { cva } from "class-variance-authority"

export { default as Badge } from "./Badge.vue"

export const badgeVariants = cva(
  "inline-flex h-[18px] items-center justify-center gap-1 rounded-sm border border-transparent px-1.5 text-2xs font-medium w-fit whitespace-nowrap shrink-0 [&>svg]:size-3 [&>svg]:pointer-events-none overflow-hidden",
  {
    variants: {
      variant: {
        default: "bg-surface-hover text-muted",
        secondary: "bg-surface-hover text-muted",
        success: "bg-success/15 text-success",
        warning: "bg-warning/15 text-warning",
        destructive: "bg-danger/15 text-danger",
        outline: "border-border text-muted",
      },
    },
    defaultVariants: { variant: "default" },
  },
)
export type BadgeVariants = VariantProps<typeof badgeVariants>

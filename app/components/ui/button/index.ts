import type { VariantProps } from "class-variance-authority"
import { cva } from "class-variance-authority"

export { default as Button } from "./Button.vue"

// Spec: replica/design/components.md > Button. Focus ring is the global :focus-visible outline from tokens.css.
export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-md border border-transparent text-sm font-medium transition-colors duration-100 active:translate-y-px disabled:pointer-events-none disabled:opacity-50 aria-busy:pointer-events-none [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-accent-hover",
        secondary: "bg-surface-hover text-text border-border hover:border-border-input",
        outline: "bg-transparent text-text border-border hover:bg-surface-hover",
        ghost: "bg-transparent text-muted hover:bg-surface-hover hover:text-text",
        destructive: "bg-danger-solid text-on-danger hover:opacity-90",
        link: "text-accent underline-offset-4 hover:text-accent-hover hover:underline h-auto px-0",
      },
      size: {
        "default": "h-8 px-3",
        "xs": "h-6 px-2 text-xs [&_svg:not([class*='size-'])]:size-3.5",
        "sm": "h-7 px-3",
        "lg": "h-10 px-4 text-base",
        "icon": "size-8",
        "icon-xs": "size-6 [&_svg:not([class*='size-'])]:size-3.5",
        "icon-sm": "size-7",
        "icon-lg": "size-10",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
)
export type ButtonVariants = VariantProps<typeof buttonVariants>

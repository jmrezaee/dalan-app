// Accessibility findings we accept for now, each with a reason. An empty list is the goal.
export interface Allowed { rule: string, state?: string, selector?: string, reason: string }

export const ALLOWED: Allowed[] = [
  {
    rule: 'aria-required-children',
    selector: '.overflow-x-auto',
    reason: 'The tab bar is a tablist whose closable tabs sit next to their own close button. Keyboard users close a tab with the shortcut (Mod+W). Revisit with a roving-tabindex tab list and a Delete key.',
  },
  {
    rule: 'aria-hidden-focus',
    state: 'S02-context-menu',
    reason: 'Reka UI hides the page behind an open context menu and leaves focus on the page until the menu takes it. The menu itself is fully reachable by keyboard.',
  },
]

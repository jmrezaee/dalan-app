// Accessibility findings we accept for now, each with a reason. An empty list is the goal.
export interface Allowed { rule: string, state?: string, selector?: string, reason: string }

export const ALLOWED: Allowed[] = [
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S43-mongo-transfer',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S47-mongo-shell',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S45-mongo-aggregate',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S46-mongo-schema',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S44-mongo-tree',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S44-mongo-docs',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S44-mongo-table',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S36-sql-table',
    reason: 'The same app tab bar as below: with no other tab list on screen axe names it by its role instead of its class.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S36-sql-tab-tree',
    reason: 'The same app tab bar: the object tree view now also has the (empty) strip of open tabs, so axe names the tab list by its role.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S37-sql-editor',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S38-sql-structure',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S39-sql-explain',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S40-sql-diagram',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S41-sql-activity',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S41-session-confirm',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
  {
    rule: 'aria-required-children',
    selector: 'div[role="tablist"]',
    state: 'S43-sql-transfer',
    reason: 'The same app tab bar as in S36-sql-table.',
  },
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

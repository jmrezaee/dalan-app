import { formatCombo } from '~/utils/shortcuts'

/**
 * Labels for shortcuts, from what is in force now: the user's own bindings, and Ctrl instead of ⌘ off macOS.
 * `label('new-tab')` for a rebindable action, `fixed('Mod+S')` for one that never changes.
 */
export function useShortcutLabel() {
  const settings = useSettingsStore()
  return {
    label: (id: string) => formatCombo(settings.combos[id] ?? ''),
    fixed: (combo: string) => formatCombo(combo),
  }
}

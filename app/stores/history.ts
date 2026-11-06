import { defineStore } from 'pinia'
import { useTauri } from '~/composables/useTauri'

/** Command history and the AI helper for the terminal side panel (S20). The components call these, never `useTauri()`. */
export const useHistoryStore = defineStore('history', () => {
  const api = useTauri()
  /** Snippet suggestions the user waved away ("Not now"), until the app restarts. */
  const dismissedHints = reactive(new Set<string>())
  return { dismissedHints, aiSuggest: api.aiSuggest, historyList: api.historyList, historyClear: api.historyClear }
})

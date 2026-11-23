<script setup lang="ts">
// The SQL editor (S37): CodeMirror 6 themed from the design tokens, with completion from the catalog Rust read.
// It lives as long as its query tab (hard rule 4), so the caret, undo history and scroll survive switching tabs.
import { Compartment, EditorState, Prec } from '@codemirror/state'
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { PostgreSQL, sql } from '@codemirror/lang-sql'
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete'
import { HighlightStyle, bracketMatching, indentOnInput, syntaxHighlighting } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'
import type { SqlCatalog } from '~/types/bindings'

const props = defineProps<{ modelValue: string, catalog: SqlCatalog | null, label: string }>()
const emit = defineEmits<{
  'update:modelValue': [text: string]
  'cursor': [pos: number]
  'run': [mode: 'current' | 'selection' | 'all', selected: string]
  'save': []
  'format': []
}>()

const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null
const language = new Compartment()

/** The syntax colours are the terminal's ANSI tokens, so the editor follows the theme. */
const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier], color: 'var(--ansi-magenta)' },
  { tag: [t.string, t.special(t.string)], color: 'var(--ansi-green)' },
  { tag: [t.number, t.bool, t.null], color: 'var(--ansi-yellow)' },
  { tag: [t.function(t.variableName), t.standard(t.name), t.typeName], color: 'var(--ansi-blue)' },
  { tag: [t.comment, t.lineComment, t.blockComment], color: 'var(--ansi-bright-black)', fontStyle: 'italic' },
  { tag: [t.operator, t.punctuation], color: 'var(--text-muted)' },
])

const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--bg)', color: 'var(--text)', fontSize: '12px' },
  '.cm-scroller': { fontFamily: 'var(--font-code)', lineHeight: '20px', overflow: 'auto' },
  '.cm-content': { caretColor: 'var(--accent)', padding: '8px 0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused': { outline: '2px solid var(--accent)', outlineOffset: '-2px' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'color-mix(in srgb, var(--accent) 22%, transparent)' },
  '.cm-gutters': { backgroundColor: 'var(--bg)', color: 'var(--text-muted)', border: 'none', borderRight: '1px solid var(--border)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--surface) 70%, transparent)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--surface)', color: 'var(--text)' },
  '.cm-tooltip': { backgroundColor: 'var(--surface-raised)', border: '1px solid var(--border)', borderRadius: '8px', color: 'var(--text)' },
  '.cm-tooltip-autocomplete > ul': { fontFamily: 'var(--font-code)', fontSize: '12px' },
  '.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'var(--surface-hover)', color: 'var(--text)' },
  '.cm-completionDetail': { color: 'var(--text-muted)' },
  '.cm-matchingBracket': { backgroundColor: 'color-mix(in srgb, var(--accent) 25%, transparent)' },
})

/** `orders` and `public.orders` both complete to the table's columns; a table in the public schema wins a bare name. */
function languageFor(catalog: SqlCatalog | null) {
  const schema: Record<string, string[]> = {}
  for (const tb of [...(catalog?.tables ?? [])].sort((a, b) => Number(a.schema === 'public') - Number(b.schema === 'public'))) {
    schema[`${tb.schema}.${tb.name}`] = tb.columns
    schema[tb.name] = tb.columns
  }
  return sql({ dialect: PostgreSQL, schema, upperCaseKeywords: false })
}

const selected = () => (view ? view.state.sliceDoc(view.state.selection.main.from, view.state.selection.main.to) : '')

onMounted(() => {
  view = new EditorView({
    parent: host.value!,
    state: EditorState.create({
      doc: props.modelValue,
      extensions: [
        // Ours first: ⌘↵ and friends must win over the default keymap.
        Prec.highest(keymap.of([
          { key: 'Mod-Enter', run: () => { const s = selected(); emit('run', s ? 'selection' : 'current', s); return true } },
          { key: 'Shift-Mod-Enter', run: () => { emit('run', 'all', ''); return true } },
          { key: 'Mod-s', run: () => { emit('save'); return true } },
          { key: 'Alt-Shift-f', run: () => { emit('format'); return true } },
        ])),
        lineNumbers(), highlightActiveLineGutter(), highlightActiveLine(), drawSelection(), history(), indentOnInput(), bracketMatching(), closeBrackets(),
        autocompletion({ activateOnTyping: true }),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap, indentWithTab]),
        language.of(languageFor(props.catalog)),
        syntaxHighlighting(highlight),
        theme,
        EditorView.contentAttributes.of({ 'aria-label': props.label, 'aria-multiline': 'true', 'spellcheck': 'false', 'autocapitalize': 'off' }),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) emit('update:modelValue', u.state.doc.toString())
          if (u.selectionSet || u.docChanged) emit('cursor', u.state.selection.main.head)
        }),
      ],
    }),
  })
})
onBeforeUnmount(() => view?.destroy())

// Text set from outside (Format, History, a saved query) replaces the document; typing already matches it.
watch(() => props.modelValue, (v) => {
  if (view && v !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: v } })
})
watch(() => props.catalog, c => view?.dispatch({ effects: language.reconfigure(languageFor(c)) }))

defineExpose({ focus: () => view?.focus(), selected })
</script>

<template>
  <div ref="host" class="min-h-0 flex-1 overflow-hidden" :data-testid="tid('sql-editor')" />
</template>

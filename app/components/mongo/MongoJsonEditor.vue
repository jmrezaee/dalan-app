<script setup lang="ts">
// A JSON editor for one document (S44): CodeMirror 6 themed from the tokens, canonical extended JSON so every type is
// visible. Mod-Enter saves, Esc is left to the dialog.
import { EditorState, Prec } from '@codemirror/state'
import { EditorView, drawSelection, highlightActiveLine, keymap, lineNumbers } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { json } from '@codemirror/lang-json'
import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
import { HighlightStyle, bracketMatching, syntaxHighlighting } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'

const props = defineProps<{ modelValue: string, label: string }>()
const emit = defineEmits<{ 'update:modelValue': [text: string], 'save': [] }>()
const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null

const highlight = HighlightStyle.define([
  { tag: [t.propertyName], color: 'var(--ansi-blue)' },
  { tag: [t.string], color: 'var(--ansi-green)' },
  { tag: [t.number, t.bool, t.null], color: 'var(--ansi-yellow)' },
  { tag: [t.punctuation, t.brace, t.squareBracket], color: 'var(--text-muted)' },
])
const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--bg)', color: 'var(--text)', fontSize: '12px' },
  '.cm-scroller': { fontFamily: 'var(--font-code)', lineHeight: '20px', overflow: 'auto' },
  '.cm-content': { caretColor: 'var(--accent)', padding: '8px 0' },
  '&.cm-focused': { outline: '2px solid var(--accent)', outlineOffset: '-2px' },
  '.cm-gutters': { backgroundColor: 'var(--bg)', color: 'var(--text-muted)', border: 'none', borderRight: '1px solid var(--border)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--surface) 70%, transparent)' },
  '.cm-matchingBracket': { backgroundColor: 'color-mix(in srgb, var(--accent) 25%, transparent)' },
})

onMounted(() => {
  view = new EditorView({
    parent: host.value!,
    state: EditorState.create({
      doc: props.modelValue,
      extensions: [
        Prec.highest(keymap.of([{ key: 'Mod-Enter', run: () => { emit('save'); return true } }])),
        lineNumbers(), highlightActiveLine(), drawSelection(), history(), bracketMatching(), closeBrackets(), json(),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, indentWithTab]),
        syntaxHighlighting(highlight), theme,
        EditorView.contentAttributes.of({ 'aria-label': props.label, 'aria-multiline': 'true', 'spellcheck': 'false', 'autocapitalize': 'off' }),
        EditorView.updateListener.of((u) => { if (u.docChanged) emit('update:modelValue', u.state.doc.toString()) }),
      ],
    }),
  })
})
onBeforeUnmount(() => view?.destroy())
watch(() => props.modelValue, (v) => {
  if (view && v !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: v } })
})
defineExpose({ focus: () => view?.focus() })
</script>

<template>
  <div ref="host" class="h-full min-h-0 overflow-hidden rounded-md border border-border" :data-testid="tid('mongo-editor')" />
</template>

import type { Ref } from 'vue'

/**
 * Draws only the rows near the viewport of a long list (the key tree can hold 100,000 scanned rows).
 * Put `scroller` on the scrolling element and call `onScroll` from its scroll event; render rows `from` to `to`
 * with a spacer of `padTop` px above and `padBottom` px below.
 */
export function useVirtualRows(count: Ref<number>, rowHeight: number, overscan = 12) {
  const scroller = ref<HTMLElement | null>(null)
  const scrollTop = ref(0)
  const viewHeight = ref(600)
  function onScroll() {
    if (!scroller.value) return
    scrollTop.value = scroller.value.scrollTop
    viewHeight.value = scroller.value.clientHeight || 600
  }
  const from = computed(() => Math.max(0, Math.floor(scrollTop.value / rowHeight) - overscan))
  const to = computed(() => Math.min(count.value, from.value + Math.ceil(viewHeight.value / rowHeight) + overscan * 2))
  const padTop = computed(() => from.value * rowHeight)
  const padBottom = computed(() => Math.max(0, count.value - to.value) * rowHeight)
  /** Scrolls so that row `i` is in view (keyboard moves). */
  function reveal(i: number) {
    const el = scroller.value
    if (!el) return
    const top = i * rowHeight
    if (top < el.scrollTop) el.scrollTop = top
    else if (top + rowHeight > el.scrollTop + el.clientHeight) el.scrollTop = top + rowHeight - el.clientHeight
    onScroll()
  }
  onMounted(() => nextTick(onScroll))
  // A list that got shorter than where we are scrolled to goes back to the top.
  watch(count, (n) => { if (scroller.value && scroller.value.scrollTop > n * rowHeight) scroller.value.scrollTop = 0; nextTick(onScroll) })
  return { scroller, onScroll, from, to, padTop, padBottom, reveal }
}

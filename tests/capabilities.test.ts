// What the page asks Tauri to do must be allowed by src-tauri/capabilities/default.json, or it fails only in the real
// window (a refused permission is an error nobody sees in Chromium or in the fake backend). Found by hand on macOS:
// `data-tauri-drag-region` needs `core:window:allow-start-dragging`, which `core:default` does not include.
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const root = process.cwd()
const granted: string[] = JSON.parse(readFileSync(join(root, 'src-tauri/capabilities/default.json'), 'utf8')).permissions

function sources(dir: string): string[] {
  return readdirSync(join(root, dir)).flatMap((name) => {
    const rel = join(dir, name)
    if (['node_modules', '.nuxt', '.output', 'types'].includes(name)) return []
    if (statSync(join(root, rel)).isDirectory()) return sources(rel)
    return /\.(vue|ts)$/.test(name) ? [rel] : []
  })
}
const code = sources('app').map(f => readFileSync(join(root, f), 'utf8')).join('\n')

describe('capabilities cover what the page uses', () => {
  it('a draggable title bar needs start-dragging', () => {
    if (code.includes('data-tauri-drag-region')) expect(granted).toContain('core:window:allow-start-dragging')
  })

  it('every Tauri plugin the page imports has a permission', () => {
    for (const m of code.matchAll(/from '@tauri-apps\/plugin-([\w-]+)'/g)) {
      expect(granted.some(p => p.startsWith(`${m[1]}:`)), `no "${m[1]}:" permission in default.json`).toBe(true)
    }
  })

  it('window and webview methods called from the page are granted one by one', () => {
    const used = [...code.matchAll(/(?:getCurrentWindow|getCurrentWebviewWindow|getCurrentWebview)\(\)\.(\w+)\(/g)].map(m => m[1]!)
    for (const method of used) {
      const kebab = method.replace(/[A-Z]/g, c => `-${c.toLowerCase()}`)
      expect(granted.some(p => p === `core:window:allow-${kebab}` || p === `core:webview:allow-${kebab}`), `${method} needs core:window:allow-${kebab}`).toBe(true)
    }
  })

  it('stays minimal: no filesystem, http or shell permission for the webview', () => {
    expect(granted.filter(p => /^(fs|http|shell)[:-]/.test(p))).toEqual([])
  })
})

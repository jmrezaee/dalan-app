// CLAUDE.md's hard rules, as tests that fail when a rule is broken (replica/testing/test-plan.md section 9.1).
// Browser and Rust parts of the same rules live in tests/e2e/rules.spec.ts and src-tauri/src/*/tests.rs.
import { describe, expect, it } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const root = process.cwd()
const read = (p: string) => readFileSync(join(root, p), 'utf8')
const bindings = read('app/types/bindings.ts')

/** All files under a folder with one of the extensions, skipping generated and vendored folders. */
function files(dir: string, exts: string[], skip = ['node_modules', '.nuxt', '.output', '.output-test', 'target', 'ui']): string[] {
  return readdirSync(join(root, dir)).flatMap((name) => {
    const rel = join(dir, name)
    if (skip.includes(name)) return []
    if (statSync(join(root, rel)).isDirectory()) return files(rel, exts, skip)
    return exts.some(e => name.endsWith(e)) ? [rel] : []
  })
}

// ---- the generated types: type name -> body, and the command return types
const typeBodies = new Map<string, string>()
for (const m of bindings.matchAll(/export type (\w+) =\s*([\s\S]*?);\n(?=\n|export|\/\*|\/\*\*)/g)) typeBodies.set(m[1]!, m[2]!)
const returnTypes = [...bindings.matchAll(/typedError<([^,]+?(?:\{[^}]*\})?[^,]*?), AppError>/g)].map(m => m[1]!)

describe('rule 1: secrets never reach JavaScript', () => {
  // A field with one of these names would carry a secret to the UI. The only exception is named, with its reason.
  const FORBIDDEN = /^(password|passphrase|private_key|secret|api_key|token)$/
  const ALLOWED = new Set(['Inherited_Serialize.password']) // the label of the group that supplies a saved password, never the password

  function reachable(): Map<string, string> {
    const seen = new Map<string, string>()
    const queue = returnTypes.flatMap(t => [...t.matchAll(/\b[A-Z]\w+\b/g)].map(m => m[0]))
    while (queue.length) {
      const name = queue.pop()!
      if (seen.has(name) || !typeBodies.has(name)) continue
      const body = typeBodies.get(name)!
      seen.set(name, body)
      for (const m of body.matchAll(/\b[A-Z]\w+\b/g)) queue.push(m[0])
    }
    return seen
  }

  it('no type a command returns has a field that could hold a secret', () => {
    const reach = reachable()
    expect(reach.size).toBeGreaterThan(40) // the walk found the real set of types
    const bad: string[] = []
    for (const [name, body] of reach) {
      for (const f of body.matchAll(/^\s*([a-z_]+)\??:/gm)) {
        if (FORBIDDEN.test(f[1]!) && !ALLOWED.has(`${name}.${f[1]}`)) bad.push(`${name}.${f[1]}`)
      }
    }
    expect(bad).toEqual([])
  })

  it('the secret-carrying input types are never returned', () => {
    const reach = reachable()
    for (const input of ['HostInput', 'GroupInput', 'IdentityInput', 'GenerateInput', 'ImportInput', 'AiSettings', 'KeyUpdate']) {
      for (const suffix of ['', '_Serialize', '_Deserialize']) {
        // `_Deserialize` and plain input types exist to receive secrets from the UI; none may be reachable from a return type.
        if (suffix !== '_Serialize') expect(reach.has(`${input}${suffix}`), `${input}${suffix} is returned by a command`).toBe(false)
      }
    }
  })
})

describe('rule 2: the webview has no files, network or shell', () => {
  it('the capability file only grants the allowed permissions', () => {
    const allowed = new Set(['core:default', 'dialog:allow-open'])
    for (const f of readdirSync(join(root, 'src-tauri/capabilities')).filter(n => n.endsWith('.json'))) {
      const cap = JSON.parse(read(`src-tauri/capabilities/${f}`)) as { permissions: unknown[] }
      for (const p of cap.permissions) expect(allowed.has(p as string), `${f} grants ${JSON.stringify(p)}`).toBe(true)
    }
  })

  it('Cargo.toml has no fs, http or shell plugin', () => {
    const cargo = read('src-tauri/Cargo.toml')
    for (const plugin of ['tauri-plugin-fs', 'tauri-plugin-http', 'tauri-plugin-shell']) expect(cargo).not.toContain(plugin)
  })

  it('the content security policy keeps scripts and connections to the app itself', () => {
    const conf = JSON.parse(read('src-tauri/tauri.conf.json')) as { app: { security: { csp: Record<string, string> | string } } }
    const csp = conf.app.security.csp
    const text = typeof csp === 'string' ? csp : Object.entries(csp).map(([k, v]) => `${k} ${v}`).join('; ')
    expect(text).toContain("default-src")
    expect(text).not.toMatch(/connect-src[^;]*https?:\/\/(?!ipc\.localhost)/)
  })
})

describe('rule 5: host keys are always verified', () => {
  it('no setting, command or UI text offers to accept every host key', () => {
    const hits: string[] = []
    for (const f of [...files('app', ['.ts', '.vue']), ...files('src-tauri/src', ['.rs'])]) {
      if (f.endsWith('bindings.ts') || f.endsWith('known_hosts.rs')) continue // the Rust module says in a comment that there is none
      if (/accept[\s_-]?all|trust[\s_-]?all|StrictHostKeyChecking|skip[\s_-]?host[\s_-]?key/i.test(read(f))) hits.push(f)
    }
    expect(hits).toEqual([])
  })
})

describe('rule 6: types are generated', () => {
  const names = new Set([...typeBodies.keys()].map(n => n.replace(/_(Serialize|Deserialize)$/, '')))

  it('no hand-written type or interface repeats a name from the bindings', () => {
    const dupes: string[] = []
    for (const f of files('app', ['.ts', '.vue'])) {
      if (f.endsWith('bindings.ts')) continue
      for (const m of read(f).matchAll(/^\s*(?:export\s+)?(?:interface|type)\s+(\w+)\b/gm)) {
        if (names.has(m[1]!)) dupes.push(`${f}: ${m[1]}`)
      }
    }
    expect(dupes).toEqual([])
  })

  it('the UI reaches Rust only through useTauri and the generated commands', () => {
    const direct: string[] = []
    for (const f of files('app', ['.ts', '.vue'])) {
      if (f.endsWith('bindings.ts') || f.endsWith('composables/useTauri.ts')) continue
      if (/from ['"]@tauri-apps\/api\/core['"]/.test(read(f)) && !/Channel/.test(read(f))) direct.push(f)
      if (/\binvoke\(/.test(read(f)) && !f.includes('useTauri')) direct.push(f)
    }
    expect(direct).toEqual([])
  })
})

// Checks replica/features.csv (replica/testing/test-plan.md section 13):
//   - every row has the 8 columns (feature, area, priority, original, clone, test, checked, notes)
//   - `clone` is yes, partial, no or skip
//   - a `yes` row names an automated test or a dated hand check
//   - every test it names exists:  walk:ch05  e2e:overlay  fake:controls  a11y  visual  app:smoke  unit:quickConnect
//     rust:groups.rs (a file under src-tauri/src)  l3:<name of a Rust test function>
//   - `checked` is empty or a date (YYYY-MM-DD)
// Run in CI (static checks). Exit 1 on any problem.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

const root = process.cwd()
const text = readFileSync(join(root, 'replica/features.csv'), 'utf8')

/** A small CSV reader: quoted fields may hold commas and doubled quotes. */
function parse(s) {
  const rows = []
  let row = [], field = '', q = false
  for (let i = 0; i < s.length; i++) {
    const c = s[i]
    if (q) {
      if (c === '"' && s[i + 1] === '"') { field += '"'; i++ }
      else if (c === '"') q = false
      else field += c
    }
    else if (c === '"') q = true
    else if (c === ',') { row.push(field); field = '' }
    else if (c === '\n') { row.push(field); rows.push(row); row = []; field = '' }
    else field += c
  }
  if (field || row.length) { row.push(field); rows.push(row) }
  return rows
}

const walk = (dir, ext) => readdirSync(join(root, dir)).flatMap((n) => {
  const rel = join(dir, n)
  if (['node_modules', 'target'].includes(n)) return []
  return statSync(join(root, rel)).isDirectory() ? walk(rel, ext) : n.endsWith(ext) ? [rel] : []
})
let rustSource
const rustHas = (name) => {
  rustSource ??= walk('src-tauri/src', '.rs').map(f => readFileSync(join(root, f), 'utf8')).join('\n')
  return new RegExp(`fn ${name}\\b`).test(rustSource)
}

function resolves(ref) {
  const [kind, name = ''] = ref.split(':')
  const has = p => existsSync(join(root, p))
  switch (kind) {
    case 'walk': return readdirSync(join(root, 'tests/walk')).some(f => f.startsWith(`${name}-`) && f.endsWith('.spec.ts'))
    case 'e2e': return has(`tests/e2e/${name}.spec.ts`)
    case 'fake': return has(`tests/fake/${name}.spec.ts`)
    case 'a11y': return has('tests/a11y/axe.spec.ts')
    case 'visual': return has('tests/visual/screens.spec.ts')
    case 'app': return has(`tests/app/specs/${name}.e2e.ts`)
    case 'unit': return has(`tests/${name}.test.ts`)
    case 'rust': return has(`src-tauri/src/${name}`) && true
    case 'l3': return rustHas(name)
    default: return false
  }
}

const rows = parse(text).filter(r => r.length > 1 || r[0])
const [head, ...body] = rows
const problems = []
const WIDTH = 8
if (head.length !== WIDTH || head[5] !== 'test' || head[6] !== 'checked') problems.push(`the header must be feature,area,priority,original,clone,test,checked,notes (found ${head.join(',')})`)
body.forEach((r, i) => {
  const line = i + 2
  if (r.length !== WIDTH) return void problems.push(`row ${line} (${r[0]}): ${r.length} columns, expected ${WIDTH}`)
  const [feature, , , , clone, test, checked] = r
  if (!['yes', 'partial', 'no', 'skip'].includes(clone)) problems.push(`row ${line} (${feature}): clone is "${clone}"`)
  if (clone === 'yes' && !test && !checked) problems.push(`row ${line} (${feature}): "yes" needs a test or a dated hand check`)
  if (checked && !/^\d{4}-\d{2}-\d{2}$/.test(checked)) problems.push(`row ${line} (${feature}): checked "${checked}" is not a date`)
  for (const ref of test.split(';').filter(Boolean)) if (!resolves(ref)) problems.push(`row ${line} (${feature}): test "${ref}" does not exist`)
})

const yes = body.filter(r => r[4] === 'yes').length
if (problems.length) {
  console.error(problems.join('\n'))
  process.exit(1)
}
console.log(`features.csv: ${body.length} rows, ${yes} yes, all with a test or a check.`)

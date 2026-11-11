// Fails when the vendored NSIS template (src-tauri/windows/UPSTREAM.md) is out of step with the installed Tauri CLI:
//   - the `tauri-cli` version in UPSTREAM.md must be the `@tauri-apps/cli` version locked in pnpm-lock.yaml
//   - installer.upstream.nsi and English.upstream.nsh must still be the untouched upstream files (sha256 in UPSTREAM.md)
// Run: pnpm check:nsis (also in CI). Exit 1 on any problem.
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'

const read = f => readFileSync(f, 'utf8')
const upstream = read('src-tauri/windows/UPSTREAM.md')
const lock = read('pnpm-lock.yaml')
const problems = []

const noted = upstream.match(/^tauri-cli: (\S+)/m)?.[1]
const locked = lock.match(/^ {2}'@tauri-apps\/cli@(\d+\.\d+\.\d+)':/m)?.[1]
if (!noted) problems.push('UPSTREAM.md has no "tauri-cli: <version>" line.')
else if (!locked) problems.push('pnpm-lock.yaml has no @tauri-apps/cli entry.')
else if (noted !== locked) problems.push(`The template is for tauri-cli ${noted}, but pnpm-lock.yaml has ${locked}. Merge the new upstream template (see UPSTREAM.md).`)

for (const f of ['installer.upstream.nsi', 'English.upstream.nsh']) {
  const want = upstream.match(new RegExp(`^${f.replace('.', '\\.')} sha256: ([0-9a-f]{64})`, 'm'))?.[1]
  const got = createHash('sha256').update(readFileSync(`src-tauri/windows/${f}`)).digest('hex')
  if (!want) problems.push(`UPSTREAM.md has no sha256 for ${f}.`)
  else if (want !== got) problems.push(`${f} was edited (sha256 differs from UPSTREAM.md). It must stay the untouched upstream file.`)
}

if (problems.length) {
  for (const p of problems) console.error(`check-nsis-template: ${p}`)
  process.exit(1)
}
console.log(`check-nsis-template: template matches tauri-cli ${noted}.`)

// Builds walk-report/index.html from test-results/walk: every step in order with its screenshot, flow and screen
// chips, pass or fail, the time it took, and the difference from the saved baseline when there is one.
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import pixelmatch from 'pixelmatch'
import { PNG } from 'pngjs'

const root = process.cwd()
const src = join(root, 'walk-results')
const out = join(root, 'walk-report')
const baselines = join(root, 'tests', 'walk', 'baselines')
const jsonl = join(src, 'steps.jsonl')
if (!existsSync(jsonl)) {
  console.error('No walk results found. Run `pnpm walk` first.')
  process.exit(1)
}
mkdirSync(join(out, 'img'), { recursive: true })
mkdirSync(baselines, { recursive: true })
const update = process.env.WALK_UPDATE === '1'

const steps = readFileSync(jsonl, 'utf8').trim().split('\n').map(l => JSON.parse(l))
for (const s of steps) {
  s.diff = null
  if (!s.shot) continue
  copyFileSync(join(src, s.shot), join(out, 'img', s.shot))
  const base = join(baselines, `${s.theme}-${s.shot}`)
  if (update || !existsSync(base)) {
    if (update) copyFileSync(join(src, s.shot), base)
    continue
  }
  const a = PNG.sync.read(readFileSync(join(src, s.shot)))
  const b = PNG.sync.read(readFileSync(base))
  if (a.width !== b.width || a.height !== b.height) {
    s.diff = { percent: 100, image: null }
    continue
  }
  const d = new PNG({ width: a.width, height: a.height })
  const n = pixelmatch(a.data, b.data, d.data, a.width, a.height, { threshold: 0.1 })
  const percent = (n / (a.width * a.height)) * 100
  if (percent > 0.01) {
    const name = `diff-${s.shot}`
    writeFileSync(join(out, 'img', name), PNG.sync.write(d))
    copyFileSync(base, join(out, 'img', `base-${s.shot}`))
    s.diff = { percent, image: name, base: `base-${s.shot}` }
  }
}

const tokens = existsSync(join(root, 'app/assets/css/tokens.css')) ? readFileSync(join(root, 'app/assets/css/tokens.css'), 'utf8') : ''
const esc = t => String(t).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]))
const failed = steps.filter(s => s.status === 'fail').length
const changed = steps.filter(s => s.diff).length
const chapters = [...new Set(steps.map(s => s.chapter))]

const html = `<!doctype html>
<html lang="en" class="dark"><head><meta charset="utf-8"><title>Dalan walkthrough</title>
<style>
${tokens}
*{box-sizing:border-box}body{margin:0;display:grid;grid-template-columns:360px 1fr;height:100vh;background:var(--bg);color:var(--text);font:14px/1.4 Inter,system-ui,sans-serif}
nav{overflow:auto;border-right:1px solid var(--border);background:var(--surface);padding:12px}
nav h1{font-size:15px;margin:0 0 4px}nav p{margin:0 0 12px;color:var(--muted);font-size:12px}
nav h2{font-size:11px;text-transform:uppercase;letter-spacing:.04em;color:var(--muted);margin:14px 0 4px}
nav button{display:flex;gap:8px;align-items:center;width:100%;text-align:left;background:none;border:0;color:inherit;padding:6px 8px;border-radius:6px;cursor:pointer;font:inherit}
nav button:hover,nav button[aria-current=true]{background:var(--surface-hover)}
.dot{width:8px;height:8px;border-radius:50%;flex:none}.pass{background:var(--success)}.fail{background:var(--danger-solid,#b3261e)}
main{overflow:auto;padding:20px}main h2{margin:0 0 4px;font-size:18px}
.chips{display:flex;gap:6px;margin:6px 0 12px;flex-wrap:wrap}.chip{border:1px solid var(--border);border-radius:999px;padding:1px 8px;font-size:12px;color:var(--muted)}
img{max-width:100%;border:1px solid var(--border);border-radius:8px;display:block}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}.err{color:var(--danger,#e5484d);margin:8px 0}
</style></head><body>
<nav><h1>Dalan walkthrough</h1><p>${steps.length} steps · ${failed} failed · ${changed} changed from the baseline</p>
${chapters.map(c => `<h2>Chapter ${c}: ${esc(steps.find(s => s.chapter === c).chapterTitle)}</h2>${steps.map((s, i) => [s, i]).filter(([s]) => s.chapter === c).map(([s, i]) => `<button data-i="${i}"><span class="dot ${s.status}"></span><span>${esc(s.caption)}</span></button>`).join('')}`).join('')}
</nav><main id="view"></main>
<script>
const steps=${JSON.stringify(steps).replace(/</g, '\\u003c')};
const view=document.getElementById('view');
function show(i){const s=steps[i];document.querySelectorAll('nav button').forEach(b=>b.setAttribute('aria-current',b.dataset.i==i));
view.innerHTML='<h2>'+s.id+' · '+s.caption.replace(/[<>&]/g,'')+'</h2><div class="chips"><span class="chip">'+(s.status==='pass'?'passed':'FAILED')+'</span>'+(s.flow?'<span class="chip">'+s.flow+'</span>':'')+(s.screen?'<span class="chip">'+s.screen+'</span>':'')+'<span class="chip">'+s.ms+' ms</span><span class="chip">'+s.theme+'</span>'+(s.diff?'<span class="chip">'+s.diff.percent.toFixed(2)+'% different from the baseline</span>':'')+'</div>'+(s.error?'<p class="err">'+s.error.replace(/[<>&]/g,'')+'</p>':'')+(s.shot?(s.diff&&s.diff.image?'<div class="pair"><div><p>Now</p><img src="img/'+s.shot+'"></div><div><p>Baseline</p><img src="img/'+s.diff.base+'"></div><div><p>Difference</p><img src="img/'+s.diff.image+'"></div></div>':'<img src="img/'+s.shot+'">'):'<p>No screenshot.</p>');}
document.querySelectorAll('nav button').forEach(b=>b.onclick=()=>show(+b.dataset.i));
document.addEventListener('keydown',e=>{const cur=[...document.querySelectorAll('nav button')].findIndex(b=>b.getAttribute('aria-current')==='true');if(e.key==='ArrowDown')show(Math.min(steps.length-1,cur+1));if(e.key==='ArrowUp')show(Math.max(0,cur-1))});
show(Math.max(0,steps.findIndex(s=>s.status==='fail')));
</script></body></html>`
writeFileSync(join(out, 'index.html'), html)
console.log(`${steps.length} steps, ${failed} failed, ${changed} different from the baseline. Gallery: ${join(out, 'index.html')}`)

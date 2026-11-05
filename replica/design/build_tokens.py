import json
dark = {
 "bg":"#0f1115","surface":"#161a20","surface-raised":"#1c2128","surface-hover":"#242a33","chrome":"#0b0d10",
 "border":"#2a313b","border-input":"#5f6979",
 "text":"#e6e9ef","text-muted":"#9aa3b1",
 "accent":"#3cc8c0","accent-hover":"#66d6cf","on-accent":"#04201e",
 "danger":"#f07178","danger-solid":"#e5484d","on-danger":"#16080a",
 "success":"#86d05c","warning":"#e8b85a","on-warning":"#1a1305",
 "terminal-bg":"#0d0f13","terminal-fg":"#d7dbe2","terminal-cursor":"#3cc8c0",
 "ansi-black":"#262b33","ansi-red":"#ef6b73","ansi-green":"#8fd17f","ansi-yellow":"#e8c46a",
 "ansi-blue":"#6aa7ff","ansi-magenta":"#c99bf0","ansi-cyan":"#5fd0d8","ansi-white":"#c9ced8",
 "ansi-bright-black":"#7a8494","ansi-bright-red":"#ff8a90","ansi-bright-green":"#a9e29a","ansi-bright-yellow":"#f3d58a",
 "ansi-bright-blue":"#8dbcff","ansi-bright-magenta":"#dab6f6","ansi-bright-cyan":"#86e0e6","ansi-bright-white":"#f2f4f8",
}
light = {
 "bg":"#ffffff","surface":"#f5f6f8","surface-raised":"#ffffff","surface-hover":"#eceef2","chrome":"#eceff3",
 "border":"#dfe3e8","border-input":"#868f9c",
 "text":"#161a20","text-muted":"#5a6370",
 "accent":"#0b7a76","accent-hover":"#086561","on-accent":"#ffffff",
 "danger":"#c42b2b","danger-solid":"#c42b2b","on-danger":"#ffffff",
 "success":"#3a7a1c","warning":"#8a5a00","on-warning":"#ffffff",
 "terminal-bg":"#fbfbfc","terminal-fg":"#1f2329","terminal-cursor":"#0b7a76",
 "ansi-black":"#1f2329","ansi-red":"#c4313b","ansi-green":"#2b7a30","ansi-yellow":"#865c00",
 "ansi-blue":"#1f5fd1","ansi-magenta":"#8e3fb5","ansi-cyan":"#0e7680","ansi-white":"#5e6672",
 "ansi-bright-black":"#6b7380","ansi-bright-red":"#a3242d","ansi-bright-green":"#1f6224","ansi-bright-yellow":"#6e4b00",
 "ansi-bright-blue":"#1847a8","ansi-bright-magenta":"#73308f","ansi-bright-cyan":"#0a5f66","ansi-bright-white":"#2d333b",
}
pairs=[]
for t in ("dark","light"):
  P=lambda *a: pairs.append([f"{t}-{a[0]}",f"{t}-{a[1]}"]+list(a[2:]))
  for bg in ("bg","surface","surface-raised","surface-hover","chrome"):
    P("text",bg); P("text-muted",bg)
  for bg in ("bg","surface","surface-raised"):
    P("accent",bg); P("danger",bg); P("success",bg); P("warning",bg)
  P("on-accent","accent"); P("on-accent","accent-hover"); P("on-danger","danger-solid"); P("on-warning","warning")
  for bg in ("bg","surface"): P("border-input",bg,"ui"); P("accent",bg,"ui")
  P("terminal-fg","terminal-bg"); P("terminal-cursor","terminal-bg","ui")
  for k in (dark if t=="dark" else light):
    if k.startswith("ansi-") and not (t=="dark" and k=="ansi-black") and not (t=="light" and k=="ansi-bright-white"):
      P(k,"terminal-bg")
tokens = {
 "_about":"Design tokens for Dalan (clean-room rebuild of an SSH client). Brand: turquoise accent (Persian tile), placeholder blue retired. Role names are fixed; /replica-brand changes values only. Two themes: dark (default) and light. Check with: python3 contrast.py tokens.json. Excluded from contrast pairs on purpose: dark ansi-black and light ansi-bright-white (the 'invisible on own background' slots every terminal palette has).",
 "color":{"dark":dark,"light":light},
 "pairs":pairs,
 "font":{
   "sans":"Inter, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif",
   "mono":"'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
   "_note":"Both fonts are SIL OFL; bundle the woff2 files locally (offline desktop app, no CDN)."
 },
 "type":{
   "2xs":{"size":11,"line":14,"weight":500,"use":"badges, kbd, tab status"},
   "xs":{"size":12,"line":16,"weight":400,"use":"captions, table meta, hints"},
   "sm":{"size":13,"line":20,"weight":400,"use":"default UI text, inputs, menus"},
   "base":{"size":14,"line":20,"weight":500,"use":"card titles, emphasised rows"},
   "lg":{"size":16,"line":24,"weight":600,"use":"sheet and dialog titles"},
   "xl":{"size":20,"line":28,"weight":600,"use":"page titles"},
   "2xl":{"size":24,"line":32,"weight":700,"use":"empty-state and unlock headlines"},
   "terminal":{"size":14,"line":1.2,"weight":400,"use":"xterm default; user setting 10-24"}
 },
 "space":[0,2,4,6,8,12,16,20,24,32,40,48,64],
 "radius":{"sm":4,"md":6,"lg":10,"xl":14,"pill":999},
 "shadow":{
   "dark":{"card":"none","pop":"0 8px 24px rgba(0,0,0,.45), 0 0 0 1px rgba(255,255,255,.04)","drag":"0 12px 32px rgba(0,0,0,.55)"},
   "light":{"card":"0 1px 2px rgba(16,24,40,.06)","pop":"0 8px 24px rgba(16,24,40,.14), 0 0 0 1px rgba(16,24,40,.05)","drag":"0 12px 32px rgba(16,24,40,.20)"}
 },
 "motion":{"fast":"100ms","base":"160ms","slow":"240ms","ease":"cubic-bezier(.2,.8,.2,1)","_note":"Respect prefers-reduced-motion: drop transforms, keep opacity fades <= 100ms."},
 "layout":{
   "window-min":[960,600],
   "tabbar-height":40,"tabbar-inset-mac":78,"tab-min":120,"tab-max":220,
   "vault-nav":220,"vault-nav-collapsed":56,"vault-nav-collapse-below":1100,
   "toolbar-height":48,"sheet-width":440,"side-panel-width":320,
   "palette-width":640,"palette-max-height":420,
   "host-card-min":240,"row-height":44,"row-height-compact":28,"pane-header":28,"pane-min":[200,120],
   "content-padding":24
 },
 "z":{"base":0,"sticky":10,"dropdown":40,"sheet":50,"dialog":60,"palette":70,"toast":80,"tooltip":90}
}
json.dump(tokens,open("tokens.json","w"),indent=2)

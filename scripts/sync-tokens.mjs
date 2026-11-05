// Copies the generated design tokens into app/ so the app never imports from outside its srcDir.
import { copyFileSync } from 'node:fs'
copyFileSync('replica/design/tokens.css', 'app/assets/css/tokens.css')
copyFileSync('replica/design/terminal-themes.ts', 'app/utils/terminal-themes.ts')

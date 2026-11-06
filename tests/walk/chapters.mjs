// The run script reads the chapter list from the TypeScript file without compiling it: it only needs numbers and titles.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const text = readFileSync(fileURLToPath(new URL('./chapters.ts', import.meta.url)), 'utf8')
export const CHAPTERS = [...text.matchAll(/\{ n: (\d+), title: '([^']+)'/g)].map(m => ({ n: Number(m[1]), title: m[2] }))

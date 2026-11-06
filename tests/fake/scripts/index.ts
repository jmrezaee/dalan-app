// Scripted servers: believable terminal sessions, timed by the page clock, so walks and screenshots show a real-looking
// terminal. Each step is "wait this long, then print this".
export interface Step { after: number, text: string }

const ESC = '\x1b['
const green = (s: string) => `${ESC}32m${s}${ESC}0m`
const blue = (s: string) => `${ESC}1;34m${s}${ESC}0m`
const red = (s: string) => `${ESC}31m${s}${ESC}0m`
const PROMPT = `${green('deploy@web-1')}:${blue('~')}$ `

export const scripts: Record<string, () => Step[]> = {
  'ubuntu-login': () => [
    { after: 0, text: 'Welcome to Ubuntu 24.04.1 LTS (GNU/Linux 6.8.0-45-generic x86_64)\r\n\r\n' },
    { after: 120, text: ' * Documentation:  https://help.ubuntu.com\r\n * Support:        https://ubuntu.com/pro\r\n\r\nLast login: Thu Nov  6 09:41:02 2026 from 10.0.0.9\r\n' },
    { after: 200, text: PROMPT },
    { after: 300, text: 'git status\r\n' },
    { after: 150, text: `On branch main\r\nYour branch is up to date with 'origin/main'.\r\n\r\nnothing to commit, working tree clean\r\n${PROMPT}` },
    { after: 300, text: 'ls -la\r\n' },
    { after: 100, text: `total 24\r\ndrwxr-xr-x 4 deploy deploy 4096 Nov  6 09:41 ${blue('.')}\r\ndrwxr-xr-x 3 root   root   4096 Oct  2 11:05 ${blue('..')}\r\n-rw------- 1 deploy deploy  412 Nov  6 09:41 .bash_history\r\ndrwxr-xr-x 2 deploy deploy 4096 Oct  2 11:06 ${blue('releases')}\r\n-rwxr-xr-x 1 deploy deploy  218 Oct  9 16:20 ${green('deploy.sh')}\r\n${PROMPT}` },
  ],
  'nginx-status': () => [
    { after: 0, text: 'systemctl status nginx\r\n' },
    { after: 120, text: `${green('●')} nginx.service - A high performance web server and a reverse proxy server\r\n     Loaded: loaded (/lib/systemd/system/nginx.service; enabled)\r\n     Active: ${green('active (running)')} since Thu 2026-11-06 07:12:44 UTC; 2h 28min ago\r\n   Main PID: 812 (nginx)\r\n      Tasks: 3 (limit: 4557)\r\n${PROMPT}` },
  ],
  'apt-update': () => [
    { after: 0, text: 'sudo apt update\r\n' },
    { after: 200, text: 'Hit:1 http://archive.ubuntu.com/ubuntu noble InRelease\r\n' },
    { after: 200, text: 'Get:2 http://archive.ubuntu.com/ubuntu noble-updates InRelease [126 kB]\r\n' },
    { after: 300, text: 'Fetched 126 kB in 1s (118 kB/s)\r\nReading package lists... Done\r\n' },
    { after: 100, text: `12 packages can be upgraded. Run 'apt list --upgradable' to see them.\r\n${PROMPT}` },
  ],
  'sudo-prompt': () => [
    { after: 0, text: 'sudo systemctl restart nginx\r\n' },
    { after: 100, text: '[sudo] password for deploy: ' },
  ],
  'vim': () => [
    { after: 0, text: `${ESC}?1049h${ESC}H${ESC}2J~\r\n~\r\n~\r\n"notes.txt" 3L, 41B` },
    { after: 400, text: `${ESC}?1049l${PROMPT}` },
  ],
  'error-output': () => [{ after: 0, text: `${red('error:')} permission denied\r\n${PROMPT}` }],
  // 5 MB in 64 KB chunks, for backpressure and responsiveness tests.
  'flood': () => {
    const chunk = 'x'.repeat(63) + '\r\n'
    const line = chunk.repeat(1024) // 64 KB
    return Array.from({ length: 80 }, () => ({ after: 0, text: line }))
  },
}

export type ScriptName = keyof typeof scripts

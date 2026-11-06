// The populated vault: what a small team's Dalan looks like after a few weeks.
import type { Seed } from '../model'
import { NOW, dir, file } from './helpers'

const MIN = 60_000
const H = 60 * MIN
const D = 24 * H

export const team = (): Seed => ({
  now: NOW,
  vault: { exists: true, unlocked: false },
  groups: [
    { id: 'g-prod', label: 'Production', username: 'deploy', identity_id: 'i-deploy', jump_mode: 'custom', jump_host_ids: ['h-bastion'] },
    { id: 'g-prod-eu', label: 'Europe', parent_id: 'g-prod', port: 2200 },
    { id: 'g-staging', label: 'Staging', username: 'stage', theme: 'midnight' },
    { id: 'g-lab', label: 'Home lab' },
  ],
  hosts: [
    { id: 'h-bastion', label: 'bastion', address: 'bastion.example.com', username: 'ops', key_id: 'k-ed', group_id: null, tags: ['prod', 'edge'], os_hint: 'ubuntu', last_connected_at: NOW - 2 * H },
    { id: 'h-web1', label: 'web-1', address: '10.0.1.11', group_id: 'g-prod', tags: ['prod', 'web'], os_hint: 'ubuntu', last_connected_at: NOW - 20 * MIN },
    { id: 'h-web2', label: 'web-2', address: '10.0.1.12', group_id: 'g-prod', tags: ['prod', 'web'], os_hint: 'ubuntu', last_connected_at: NOW - D },
    { id: 'h-db1', label: 'db-1', address: '10.0.2.21', port: 5522, group_id: 'g-prod-eu', tags: ['prod', 'db'], os_hint: 'debian', last_connected_at: NOW - 3 * D },
    { id: 'h-stage', label: 'stage-app', address: 'stage.example.com', group_id: 'g-staging', tags: ['stage'], os_hint: 'alpine' },
    { id: 'h-nas', label: 'nas', address: '192.168.1.40', username: 'admin', group_id: 'g-lab', tags: ['home'], os_hint: 'debian', password: true },
    { id: 'h-switch', label: 'old-switch', address: '192.168.1.2', protocol: 'telnet', group_id: 'g-lab', tags: ['home'] },
    { id: 'h-console', label: 'serial console', address: '/dev/ttyUSB0', protocol: 'serial', group_id: 'g-lab', tags: ['home'], serial: { baud: 115200, data_bits: 8, stop_bits: 1, parity: 'none', flow: 'none' } },
    { id: 'h-roam', label: 'roaming laptop', address: 'laptop.example.com', protocol: 'mosh', username: 'me', tags: ['mosh'] },
  ],
  keys: [
    { id: 'k-ed', label: 'work laptop', key_type: 'ED25519', bits: null, public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKEWORK me@laptop', fingerprint: 'SHA256:fakeWorkLaptop0001', encrypted: true, passphrase_saved: true, certificate: false, certificate_note: null },
    { id: 'k-cert', label: 'prod access', key_type: 'ED25519', bits: null, public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKECERT prod', fingerprint: 'SHA256:fakeProdCert0002', encrypted: false, passphrase_saved: false, certificate: true, certificate_note: 'Valid for 6 days' },
    { id: 'k-rsa', label: 'legacy rsa', key_type: 'RSA', bits: 4096, public_key: 'ssh-rsa AAAAB3NzaC1yc2EAAAAFAKERSA legacy', fingerprint: 'SHA256:fakeLegacyRsa0003', encrypted: true, passphrase_saved: false, certificate: false, certificate_note: null },
    { id: 'k-ecdsa', label: 'unused ecdsa', key_type: 'ECDSA', bits: 256, public_key: 'ecdsa-sha2-nistp256 AAAAE2VjZHNhFAKEECDSA unused', fingerprint: 'SHA256:fakeEcdsa0004', encrypted: false, passphrase_saved: false, certificate: false, certificate_note: null },
  ],
  identities: [
    { id: 'i-deploy', label: 'deploy', username: 'deploy', password: false, key_id: 'k-ed' },
    { id: 'i-admin', label: 'home admin', username: 'admin', password: true, key_id: null },
  ],
  packages: [{ id: 'p-ops', label: 'Operations' }, { id: 'p-deploy', label: 'Deploy' }],
  snippets: [
    { id: 's-update', label: 'Update packages', script: 'sudo apt update && sudo apt -y upgrade', package_id: 'p-ops', target_host_ids: ['h-web1', 'h-web2', 'h-db1'] },
    { id: 's-disk', label: 'Disk usage', script: 'df -h', package_id: 'p-ops', target_host_ids: [] },
    { id: 's-nginx', label: 'Nginx status', script: 'systemctl status nginx', package_id: 'p-ops', target_host_ids: [] },
    { id: 's-release', label: 'Release', script: './deploy.sh', package_id: 'p-deploy', target_host_ids: ['h-web1'] },
    { id: 's-loose', label: 'Who is on', script: 'who', package_id: null, target_host_ids: [] },
  ],
  forwards: [
    { id: 'f-db', host_id: 'h-bastion', label: 'Postgres', kind: 'local', bind_address: '127.0.0.1', bind_port: 5432, dest_address: '10.0.2.21', dest_port: 5432, autostart: false, state: 'active', connections: 2, error: null },
    { id: 'f-socks', host_id: 'h-bastion', label: 'SOCKS proxy', kind: 'dynamic', bind_address: '127.0.0.1', bind_port: 1080, dest_address: null, dest_port: null, autostart: false, state: 'reconnecting', connections: 0, error: null },
    { id: 'f-remote', host_id: 'h-web1', label: 'Demo site', kind: 'remote', bind_address: '0.0.0.0', bind_port: 8080, dest_address: '127.0.0.1', dest_port: 3000, autostart: false, state: 'error', connections: 0, error: 'The server only allows remote forwards on its own address. Ask for GatewayPorts.' },
    { id: 'f-web', host_id: 'h-stage', label: 'Staging web', kind: 'local', bind_address: '127.0.0.1', bind_port: 8081, dest_address: '127.0.0.1', dest_port: 80, autostart: false, state: 'stopped', connections: 0, error: null },
  ],
  knownHosts: [
    { id: 'kh-1', host_pattern: 'bastion.example.com', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-bastion', created_at: NOW - 30 * D, last_seen_at: NOW - 2 * H },
    { id: 'kh-2', host_pattern: '10.0.1.11', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-web-1', created_at: NOW - 30 * D, last_seen_at: NOW - 20 * MIN },
    { id: 'kh-3', host_pattern: '10.0.1.12', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-web-2', created_at: NOW - 30 * D, last_seen_at: NOW - D },
    { id: 'kh-4', host_pattern: '[10.0.2.21]:5522', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-db-1', created_at: NOW - 20 * D, last_seen_at: NOW - 3 * D },
    { id: 'kh-5', host_pattern: '192.168.1.40', key_type: 'ssh-rsa', fingerprint: 'SHA256:fake-nas', created_at: NOW - 90 * D, last_seen_at: NOW - 9 * D },
    { id: 'kh-6', host_pattern: 'stage.example.com', key_type: 'ssh-ed25519', fingerprint: 'SHA256:fake-stage', created_at: NOW - 12 * D, last_seen_at: NOW - 4 * D },
  ],
  logs: [
    { id: 'l-1', host_id: 'h-web1', host_label: 'web-1', started_at: NOW - 20 * MIN, ended_at: NOW - 5 * MIN, size_bytes: 18_400, bookmarked: true, comment: 'The nginx reload that fixed the 502s' },
    { id: 'l-2', host_id: 'h-db1', host_label: 'db-1', started_at: NOW - 3 * D, ended_at: NOW - 3 * D + 40 * MIN, size_bytes: 92_100, bookmarked: false, comment: null },
    { id: 'l-3', host_id: 'h-nas', host_label: 'nas', started_at: NOW - 9 * D, ended_at: NOW - 9 * D + 10 * MIN, size_bytes: 3_200, bookmarked: false, comment: null },
  ],
  templates: [{ id: 't-web', name: 'Web servers', mode: 'split', layout: 'grid', panes: JSON.stringify([{ host_id: 'h-web1' }, { host_id: 'h-web2' }]) }],
  history: [
    { id: 'c1', command: 'tail -f /var/log/nginx/error.log', ran_at: NOW - 2 * H, host_id: 'h-web1' },
    { id: 'c2', command: 'tail -f /var/log/nginx/error.log', ran_at: NOW - 3 * H, host_id: 'h-web1' },
    { id: 'c3', command: 'tail -f /var/log/nginx/error.log', ran_at: NOW - 4 * H, host_id: 'h-web1' },
    { id: 'c4', command: 'df -h', ran_at: NOW - 5 * H, host_id: 'h-web1' },
  ],

  settings: { logs: JSON.stringify({ enabled: true }), history: JSON.stringify({ enabled: true }) },
  files: {
    local: [dir('home', [dir('me', [file('notes.txt', 412), file('.profile', 807), file('report.pdf', 2_400_000), dir('projects')])])],
    'h-web1': [
      dir('home', [dir('me', [file('deploy.sh', 218, D, 0o755), file('app.log', 5_200_000), file('.bashrc', 3771), dir('releases', [file('v1.tgz', 9_000_000)])])]),
      dir('srv', [file('index.html', 1024)]),
      dir('private'),
    ],
  },
})

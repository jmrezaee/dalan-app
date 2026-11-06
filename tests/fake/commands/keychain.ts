import type { KeySummary_Serialize } from '../../../app/types/bindings'
import { fail, type Handlers } from '../types'

type Keys = 'keysList' | 'keyGenerate' | 'keyImport' | 'keySetCertificate' | 'keyUpdate' | 'keyDelete' | 'identitiesList' | 'identitySave' | 'identityDelete' | 'knownHostsList' | 'knownHostDelete'

const TYPES: Record<string, { key_type: string, bits: number | null }> = {
  ed25519: { key_type: 'ED25519', bits: null }, ecdsa_p256: { key_type: 'ECDSA', bits: 256 }, ecdsa_p384: { key_type: 'ECDSA', bits: 384 },
  ecdsa_p521: { key_type: 'ECDSA', bits: 521 }, rsa4096: { key_type: 'RSA', bits: 4096 },
}

export const keychain: Pick<Handlers, Keys> = {
  keysList: (_a, { m }) => m.keys,
  keyGenerate: (a, { m, id }) => {
    const i = a.input
    if (!String(i.label ?? '').trim()) fail('validation', 'Give the key a name.')
    const t = TYPES[i.kind] ?? TYPES.ed25519!
    const n = m.keys.length + 1
    const key: KeySummary_Serialize = {
      id: id('key'), label: String(i.label).trim(), key_type: t.key_type, bits: t.bits,
      public_key: `ssh-${t.key_type.toLowerCase()} AAAAFAKEKEY${n} ${String(i.label).trim()}`,
      fingerprint: `SHA256:fake-fingerprint-${n}`, encrypted: !!i.passphrase, passphrase_saved: !!i.passphrase && !!i.save_passphrase,
      certificate: false, certificate_note: null,
    }
    m.keys.push(key)
    return key
  },
  keyImport: (a, { m, id }) => {
    const i = a.input
    if (!i.pem && !i.path) fail('invalid_key', 'That isn\'t a private key Dalan can read.')
    // A pasted key whose text says "ENCRYPTED" needs its passphrase; the test secret is "right-passphrase".
    if (String(i.pem ?? '').includes('ENCRYPTED')) {
      if (!i.passphrase) fail('needs_passphrase', 'This key is protected by a passphrase.')
      if (i.passphrase !== 'right-passphrase') fail('invalid_key', 'That passphrase didn\'t unlock the key.')
    }
    const n = m.keys.length + 1
    const key: KeySummary_Serialize = {
      id: id('key'), label: String(i.label || 'Imported key'), key_type: 'ED25519', bits: null, public_key: `ssh-ed25519 AAAAIMPORTED${n} imported`,
      fingerprint: `SHA256:fake-imported-${n}`, encrypted: String(i.pem ?? '').includes('ENCRYPTED'), passphrase_saved: !!i.save_passphrase,
      certificate: false, certificate_note: null,
    }
    m.keys.push(key)
    return key
  },
  keySetCertificate: (a, { m }) => {
    const k = m.keys.find(x => x.id === a.id)
    if (!k) fail('not_found', 'That key is gone.')
    if (a.certificate === null && a.path === null) {
      k.certificate = false
      k.certificate_note = null
    }
    else {
      if (String(a.certificate ?? '').includes('expired')) fail('validation', 'That certificate has expired.')
      k.certificate = true
      k.certificate_note = 'Valid for 30 days'
    }
    return k
  },
  keyUpdate: (a, { m }) => {
    const k = m.keys.find(x => x.id === a.input.id)
    if (!k) fail('not_found', 'That key is gone.')
    k.label = a.input.label
    if (a.input.forget_passphrase) k.passphrase_saved = false
    if (a.input.save_passphrase) k.passphrase_saved = true
    return k
  },
  keyDelete: (a, { m }) => {
    if (m.hosts.some(h => h.key_id === a.id) || m.identities.some(i => i.key_id === a.id)) fail('in_use', 'A host or identity still uses this key.')
    m.keys = m.keys.filter(k => k.id !== a.id)
    return null
  },
  identitiesList: (_a, { m }) => m.identities.map(i => ({ id: i.id, label: i.label, username: i.username, has_password: i.password, key_id: i.key_id })),
  identitySave: (a, { m, id }) => {
    const i = a.input
    if (!String(i.username ?? '').trim()) fail('validation', 'Enter a username.')
    let row = i.id ? m.identities.find(x => x.id === i.id) : undefined
    if (!row) {
      row = { id: id('identity'), label: '', username: '', password: false, key_id: null }
      m.identities.push(row)
    }
    row.label = String(i.label || i.username)
    row.username = i.username
    row.key_id = i.key_id ?? null
    if (i.clear_password) row.password = false
    else if (i.password) row.password = true
    return { id: row.id, label: row.label, username: row.username, has_password: row.password, key_id: row.key_id }
  },
  identityDelete: (a, { m }) => {
    m.identities = m.identities.filter(i => i.id !== a.id)
    m.hosts.forEach((h) => { if (h.identity_id === a.id) h.identity_id = null })
    return null
  },
  knownHostsList: (_a, { m }) => m.knownHosts,
  knownHostDelete: (a, { m }) => {
    m.knownHosts = m.knownHosts.filter(k => k.id !== a.id)
    return null
  },
}

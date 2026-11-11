import { toast } from 'vue-sonner'
import { findQuickHost } from '~/utils/quickConnect'

/**
 * Connects to `user@address:port` that the person typed or that a link asked for: a saved host with the same address,
 * user and port opens as it is; otherwise the address is saved first (a connection needs a host record in the vault, and
 * it then shows under Recent and can be deleted from Hosts) and opened.
 */
export async function connectAddress(q: { username: string | null, address: string, port: number | null }) {
  const hosts = useHostsStore()
  const sessions = useSessionsStore()
  const existing = findQuickHost(hosts.hosts, q)
  if (existing) return void sessions.open(existing)
  const err = await hosts.save({ id: null, label: q.address, address: q.address, port: q.port, username: q.username, password: null, clear_password: false, group_id: null, key_id: null, identity_id: null, startup_snippet_id: null, jump_mode: 'inherit', jump_host_ids: [], tags: [] })
  if (err) return void toast.error(err)
  const saved = findQuickHost(hosts.hosts, q)
  if (saved) sessions.open(saved)
}

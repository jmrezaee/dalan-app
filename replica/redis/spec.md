# Redis in Dalan: spec

A Redis browser and console inside Dalan, for the whole team, used on real servers. It adds a **Databases** section to the vault and a **Redis tab** type next to terminal and SFTP tabs.

Decisions (Mehrdad, 2026-11-15):

- **Placement:** a vault section lists saved databases; opening one adds a tab. Also reachable from a host's menu and the palette.
- **Scope:** connections, key browser, viewer and editor for every basic type, expiry, console, server info and slow log, **Pub/Sub**, **Cluster and Sentinel**, **bulk actions**.
- **Safety:** each database has a write mode (Read-only, Confirm writes, Open). New ones start as Confirm writes. Dangerous commands always need a typed confirmation. Production databases show an amber bar.

**Design:** the "Redis" page of the UI canvas (https://claude.ai/artifact/U8iEwGC82GSXeTKYXdzanJ), seven boards: `RedisDatabases`, `RedisBrowser`, `RedisValues`, `RedisSafety`, `RedisConsole`, `RedisServer`, `RedisPubSub`. Match layout and copy from them, with tokens, never their inline hex.

**Clean room:** this is our own design. The reference for "what a Redis desktop tool does" is Redis Insight; it is named only inside `replica/`. Do not fetch, read or copy its bundle, icons, copy or layout. Redis's public command documentation is fine for behaviour. "Redis" is used in the UI as a plain description of what we connect to; do not use the Redis logo (the tab and nav icon is Lucide `Database`).

## 1. Screens and flows

New IDs continue from `recon.md`.

| ID | screen | board |
| --- | --- | --- |
| S26 | Databases (vault section): table, filter, empty state, "New database" | RedisDatabases |
| S27 | Database editor (sheet): name, environment, route, address, kind, sign-in, TLS, write mode, Test connection | RedisDatabases |
| S28 | Redis tab, Browser view: key tree on the left, value on the right | RedisBrowser |
| S29 | Value editors: string, JSON, list, set, sorted set, stream (hash is on S28) | RedisValues |
| S30 | Console view, with completion and history | RedisConsole |
| S31 | Server view: tiles, slow commands, keys by database, nodes | RedisServer |
| S32 | Pub/Sub view | RedisPubSub |
| S33 | Bulk action dialog, the write confirmations, connecting and failure states | RedisSafety |

| ID | flow |
| --- | --- |
| F13 | Save a database (direct or through a host), test it, open it in a tab |
| F14 | Find a key (pattern, type, tree), read its value in pages |
| F15 | Change something: edit, add, remove, rename, expiry, delete, new key. Always through the write gate (§4) |
| F16 | Run commands in the console |
| F17 | Listen to channels and publish |
| F18 | Bulk action on a pattern: preview, confirm, run in batches, stop |
| F19 | Open a Cluster or a Sentinel-managed database; see its nodes |

Shell changes:

- Vault nav gets **Databases** (Lucide `Database`) under Keychain, with a count.
- A Redis tab shows the `Database` icon and the database name. A production database adds a 2 px amber line on top of the tab and a "production" chip in the tab's toolbar.
- The tab toolbar (44 px): name, environment chip, route text, logical database picker (`db0 · 12,408 keys`; hidden for Cluster), the four views, latency dot, write-mode button.
- Like terminal tabs, Redis tabs stay mounted (`v-show`); the views inside one tab switch on a ref, not a route. One Pinia store instance per tab (`useRedisStore(tabKey)`), the pattern P30 used for SFTP.
- Host menu (S02): "Open Redis on this host…" opens S27 with the route pre-filled, or lists the databases already saved for that host.
- Palette (S17): saved databases appear as results ("Open cache-prod"); action "New database".
- ⌘1 Vault, then SFTP tabs, then sessions and Redis tabs in tab order (no new shortcut scheme).
- Lock (F10) closes every Redis connection and listener.

## 2. Data model: one new migration

Add the next free migration (after `0005`; never edit earlier ones). Proposed shape, adjust names to the conventions in `schema.sql` (UUID v7 text ids, `vault_id`, epoch ms):

```sql
CREATE TABLE redis_databases (
  id TEXT PRIMARY KEY,
  vault_id TEXT NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  environment TEXT NOT NULL DEFAULT 'none' CHECK (environment IN ('none','staging','production')),
  route TEXT NOT NULL CHECK (route IN ('direct','host')),
  host_id TEXT REFERENCES hosts(id) ON DELETE RESTRICT,   -- required when route = 'host'
  address TEXT NOT NULL,            -- host name or IP as seen from the route's last hop; for Sentinel, a comma list of sentinels
  port INTEGER NOT NULL DEFAULT 6379,
  kind TEXT NOT NULL DEFAULT 'single' CHECK (kind IN ('single','cluster','sentinel')),
  sentinel_master TEXT,             -- required when kind = 'sentinel'
  username TEXT,
  password_secret_id TEXT REFERENCES secrets(id),
  sentinel_password_secret_id TEXT REFERENCES secrets(id),
  tls INTEGER NOT NULL DEFAULT 0,
  tls_server_name TEXT,
  write_mode TEXT NOT NULL DEFAULT 'confirm' CHECK (write_mode IN ('read_only','confirm','open')),
  default_db INTEGER NOT NULL DEFAULT 0,
  key_separator TEXT NOT NULL DEFAULT ':',
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, last_opened_at INTEGER,
  CHECK ((route = 'host') = (host_id IS NOT NULL))
);
CREATE INDEX redis_databases_vault_idx ON redis_databases(vault_id, name);
CREATE INDEX redis_databases_host_idx ON redis_databases(host_id);
```

- Deleting a host that a database uses is refused with a message naming the databases (the `RESTRICT` above), the same way other in-use references are handled.
- Console history: a `redis_history` table shaped like `command_history` after `0005` (encrypted column, keyed by database id). Cap at 500 rows per database.
- Add a fixture database for the new migration to `tests/fixtures/db`.
- Export/import of the vault (if it lists tables) must include the new table; secrets follow the existing rules.

## 3. Rust

New module `src-tauri/src/redis/` and `commands/redis.rs` (thin). Everything Redis lives in Rust (hard rule 2); the webview only sees typed results.

### Crates (approved with this spec; add them to the Stack list in `CLAUDE.md`)

- `redis` (redis-rs, `tokio-comp`, `aio`): RESP encoding and decoding, and a multiplexed connection.
- `tokio-rustls` + `rustls-native-certs`: TLS for direct and tunnelled connections. If the tree already has a rustls stack, reuse it. No OpenSSL.

### Transport: the part that bites

A database is reached one of two ways, and both must end as one `AsyncRead + AsyncWrite` stream handed to the Redis connection:

- **Direct:** `TcpStream`, then TLS if set.
- **Through a host:** open (or reuse) the SSH session to `host_id` exactly as a terminal would, including its jump chain, then open a `direct-tcpip` channel to `address:port` and use the channel's stream. TLS, if set, runs inside that stream.

Rules:

- **Do not bind a local listener** for this. A loopback port would let any local process talk to a production Redis. Check that the `redis` crate version in use can build a connection from a caller-supplied stream (`MultiplexedConnection::new(info, stream)` or its current equivalent). If it cannot, write a small RESP2/RESP3 codec in `redis/resp.rs` (tokio-util `Framed`) rather than falling back to a listener, and say so in the phase note.
- Host sign-in reuses the S18 machinery: host-key checks (hard rule 5 applies unchanged: unknown asks, changed blocks), password, passphrase, keyboard-interactive. The Redis tab shows the steps on S33 "Connecting" and hosts the same prompts.
- The SSH session used for a database is ref-counted: closing the last Redis tab on it closes it, unless a terminal tab also uses that host. A Redis tab never shows a shell.
- Keep-alive: `PING` every 30 s on idle connections. On a drop: retry with backoff (1, 2, 4, 8, 15 s; 5 tries), show the state in the tab, and never replay a write.
- Timeouts: connect 10 s, command 30 s (the console can cancel a running command by dropping its connection and reopening).

### Connections per tab

- One command connection (multiplexed), plus one dedicated connection per tab for Pub/Sub when F17 is in use, plus short-lived ones for bulk actions if needed.
- `HELLO 3` when the server supports it, else RESP2. `CLIENT SETNAME dalan`.
- After sign-in read `INFO server` and `COMMAND` once and cache the command table (name, arity, flags, key positions) for the gate and the console's completion.
- If the server says it is a replica, open the tab read-only with the S33 notice.

### Cluster and Sentinel (F19)

The library's own cluster client opens its own sockets, which cannot go through our SSH route, so do the routing here:

- **Cluster:** read `CLUSTER SHARDS` (fall back to `CLUSTER SLOTS`), keep a slot map, hash keys with CRC16 mod 16384 honouring `{hash tags}`, keep one connection per primary opened through the same route, follow `MOVED` (refresh the map) and `ASK` (one-shot `ASKING`), at most 5 redirects. Node addresses are used as the cluster reports them, as seen from the route's last hop. `SCAN` iterates every primary and merges; the cursor returned to the UI is opaque (`node index + node cursor`). No logical database picker. Multi-key commands in the console that cross slots return the server's own error untouched.
- **Sentinel:** ask each listed sentinel `SENTINEL get-master-addr-by-name <master>` until one answers, connect to that address through the same route, verify with `ROLE`. On a drop, ask again (the primary may have moved).
- S31 "Nodes": role, address, slots or "of <primary>", state. For Sentinel list the primary, replicas and sentinels.

### Values are bytes

- Keys and values are byte strings. Never go through `String` in Rust for them. Send them to the UI as bytes plus a flag `text: bool` (valid UTF-8 with no control characters except tab and newline).
- Binary keys are shown escaped (`\xNN`) and are addressed by their bytes, not by the escaped text.
- Binary values open as hex, read-only in v1.
- Before loading a value ask its size (`STRLEN`, `HLEN`, `LLEN`, `SCARD`, `ZCARD`, `XLEN`, `MEMORY USAGE`). Strings over 1 MB load the first 64 KB with "Load all". Collections load 500 items at a time (`HSCAN`, `LRANGE`, `SSCAN`, `ZRANGE … REV` with scores, `XREVRANGE`).
- JSON: a string that parses as JSON gets the JSON view. If the server has the JSON module (`JSON.GET` in the command table), `ReJSON-RL` keys are read with it and saved with `JSON.SET`; without the module there are no such keys.
- Key listing uses `SCAN` with `MATCH`, `COUNT 500` and `TYPE` when a type filter is set. **Never `KEYS`.** The tree groups by the database's separator and is built in the UI from the scanned page; "Scan more" continues the cursor; the footer says how much has been scanned.

### Commands (thin wrappers; names can follow the existing naming style)

| command | does |
| --- | --- |
| `redis_db_list`, `redis_db_save`, `redis_db_delete` | CRUD for S26/S27. Never return the password; return `has_password`. |
| `redis_db_test` | Connects with the form's values, returns version, mode and latency, then closes. |
| `redis_open(db_id, events: Channel)` → `conn_id` | Opens the route and signs in. Emits step events for S33 and the usual host prompts. |
| `redis_close(conn_id)` | Closes connections, listeners and the SSH ref. |
| `redis_select(conn_id, index)` | Logical database (single and Sentinel only). |
| `redis_scan(conn_id, cursor, pattern, type?, count)` | One page: keys with type and time left. |
| `redis_key_info(conn_id, key)` | Type, time left, size, length, encoding. |
| `redis_value_page(conn_id, key, cursor)` | One page of the value for its type. |
| `redis_write_prepare(conn_id, op)` → `Pending` | See §4. `op` is a typed enum: set string, set/remove hash field, push/set/remove list item, add/remove set member, add/remove/rescore sorted-set member, add/delete stream entry, set/clear expiry, rename, delete key, create key, publish. |
| `redis_write_commit(pending_id, typed_name?)` | Runs a prepared write. |
| `redis_console_exec(conn_id, line)` → reply or `Pending` | Parses the line (quotes and escapes like a shell would), runs it through the gate. |
| `redis_console_history(db_id)`, `…_clear` | Encrypted history. |
| `redis_info(conn_id)`, `redis_slowlog(conn_id)`, `redis_nodes(conn_id)` | S31. `SLOWLOG RESET` goes through the gate as dangerous. |
| `redis_sub_start(conn_id, name, pattern: bool, messages: Channel)`, `redis_sub_stop` | S32. Batched like terminal output (about 8 ms or 64 KB), with backpressure; the UI keeps the last 5,000 messages. |
| `redis_bulk_preview(conn_id, pattern, type?, progress: Channel)` | Scans the whole keyspace, returns the count and a sample of 20. Cancellable. |
| `redis_bulk_run(pending_id, typed_name?, progress: Channel)`, `redis_bulk_stop` | Delete (`UNLINK`, falling back to `DEL`), set expiry, remove expiry. Batches of 500 with a short pause between; per-slot batches on Cluster. Stop keeps what is done. |
| `redis_set_mode(db_id, mode, master_password?)` | Changing the mode of a production database to a less protective one needs the master password, checked in Rust. |

Types come from tauri-specta into `app/types/bindings.ts` (hard rule 6). Errors use `AppError` (hard rule 7); add codes in the style of `architecture.md`, for example `REDIS_AUTH_FAILED`, `REDIS_UNREACHABLE`, `REDIS_TLS_UNTRUSTED`, `REDIS_REPLICA`, `REDIS_READ_ONLY_MODE`, `REDIS_CONFIRM_NEEDED`, `REDIS_TOO_LARGE`, `REDIS_CROSS_SLOT`, `REDIS_REFUSED` (the server's own error text goes in `message`).

## 4. The write gate

One function in Rust (`redis/gate.rs`) decides for **every** command, whether it comes from an editor, the console, Pub/Sub publish or a bulk action. The UI never decides and cannot skip it.

**Classify** each command as one of:

- `read`: flagged read-only in the server's command table.
- `write`: flagged write, or unknown to the table (unknown means write), or `EVAL`/`EVALSHA`/`FCALL` (their `_RO` forms are `read`), or `PUBLISH`.
- `dangerous`: `FLUSHALL`, `FLUSHDB`, `SHUTDOWN`, `CONFIG SET`, `CONFIG REWRITE`, `CONFIG RESETSTAT`, `DEBUG *`, `SWAPDB`, `REPLICAOF`/`SLAVEOF`, `FAILOVER`, `CLUSTER` subcommands that change the cluster (`ADDSLOTS`, `DELSLOTS`, `FORGET`, `MEET`, `RESET`, `FAILOVER`, `REPLICATE`, `SETSLOT`, …), `SCRIPT FLUSH`, `SCRIPT KILL`, `FUNCTION FLUSH`, `FUNCTION DELETE`, `FUNCTION RESTORE`, `ACL SETUSER`, `ACL DELUSER`, `ACL LOAD`, `ACL SAVE`, `MODULE LOAD`, `MODULE UNLOAD`, `CLIENT KILL`, `MIGRATE`, `RESTORE … REPLACE`, `SAVE`, `BGSAVE`, `BGREWRITEAOF`, `SLOWLOG RESET`, `LATENCY RESET`, any bulk action, and deleting a key with more than 10,000 items.
- `heavy`: `KEYS`, and unbounded reads of big collections (`SMEMBERS`, `HGETALL`, `LRANGE 0 -1`, `ZRANGE 0 -1` when the length is over 10,000). Not sent by default; the console offers the safe form (`SCAN`…) and "Run anyway…", which is a confirmation.
- `refused`: `MONITOR` (always: it can hurt a busy server), `SUBSCRIBE` family in the console (use the Pub/Sub view), `SYNC`/`PSYNC`, `CLIENT PAUSE`.

Inside `MULTI`, each queued command is classified as it is queued; `EXEC` takes the strongest class of its queue.

**Decide** by the database's mode:

| class | Read-only | Confirm writes | Open |
| --- | --- | --- | --- |
| read | run | run | run |
| write | refuse (`REDIS_READ_ONLY_MODE`) | confirm | run |
| heavy | confirm | confirm | confirm |
| dangerous | refuse | typed confirm | typed confirm |
| refused | refuse | refuse | refuse |

**Confirm** is two calls, so the UI cannot claim a confirmation it did not show:

1. `redis_write_prepare` (or `redis_console_exec`) returns a `Pending { id, level: 'confirm' | 'typed', command_preview, target, before?, after?, count? }`. Rust stores the exact command bytes under `id` for 2 minutes, bound to the connection.
2. `redis_write_commit(id, typed_name?)` runs those stored bytes, once. For `typed`, Rust compares `typed_name` with the database's name. A second commit, an expired id or a different connection is refused.

`command_preview` is the command as text with long arguments shortened, built in Rust. Values in the preview follow the "values are bytes" rule.

Other rules:

- In Read-only mode the editors hide their add, edit and delete controls, the console still accepts reads, and the tab shows the S33 banner. Dalan does not change any server setting to enforce this; the protection is the gate. (A team that wants a server-side guarantee as well can sign in with a read-only ACL user.)
- Dialogs: Cancel is focused; the destructive button is `danger-solid`; the typed field must match exactly.
- The environment (`production`) changes the look (amber line, chip, the word in dialogs) and requires the master password to loosen the mode. It does not change the table above.
- Console history never stores `AUTH`, `HELLO … AUTH`, `ACL SETUSER`, `CONFIG SET requirepass|masterauth|masteruser`, `MIGRATE … AUTH`. The password field in S27 goes UI → Rust once (hard rule 1).
- Every committed write and every refusal is written to the app log (database name, class, command name only, no arguments).

## 5. UI

Components under `app/components/redis/`: `DatabasesView`, `DatabaseSheet`, `RedisTab`, `KeyTree`, `KeyHeader`, `ValueHash`, `ValueString`, `ValueJson`, `ValueList`, `ValueSet`, `ValueZset`, `ValueStream`, `RedisConsole`, `RedisServer`, `RedisPubSub`, `BulkDialog`, `WriteConfirmDialog`, `DangerConfirmDialog`, `ConnectSteps`. Stores: `redisDatabases` (the list) and `useRedisStore(tabKey)` (one per tab). Components reach Rust through stores only.

Add each new component to `replica/design/components.md` with its states (loading, empty, error, read-only, disabled) and keyboard behaviour, in the same format as the others, **before** building it. Notes that must be in there:

- **Type chips:** `str`, `hash`, `list`, `set`, `zset`, `stream`, `json`, coloured with the ANSI tokens (blue, magenta, green, yellow, red, cyan, white). If a new token is needed, add it to `tokens.json` and run the contrast check. The chip always carries the text, so colour is never the only signal.
- **Key tree:** virtualised (the list can hold 100,000 scanned rows); arrow keys move, Right/Left open and close a folder, Enter opens a key, typing filters; `role="tree"`.
- **Value tables:** virtualised; Enter edits the focused row, Esc cancels, the edit ends in "Review change" (Confirm mode) or "Save" (Open mode).
- **Expiry control:** shows "Expires in 23h 14m" or "No expiry"; editing accepts `90s`, `15m`, `2h`, `7d` or a date; "Remove expiry".
- **Console:** our own input and output (not xterm): monospace, output selectable, ↑/↓ history, Tab completes command names and shows the syntax line from the cached command table, ⌘L clears, replies printed in a reading-friendly form with types (`(integer)`, quoted strings, numbered lists, nested lists indented). Long replies are cut at 1,000 lines with "Show all".
- **Pub/Sub:** Pause keeps receiving into a 5,000-message buffer and shows how many were dropped; the list is virtualised.
- **Server:** refresh every 5 s only while the view is visible and the window is focused.
- **Empty states** (our copy): Databases: "No databases yet. Add one to browse its keys, run commands and watch channels." Browser with nothing selected: "Pick a key to see its value." No match: "No keys match user:* in what's been scanned so far. Scan more?"
- All copy is ours, in the brand voice (calm, precise, protective). The strings on the canvas are the starting point.

Settings (S23): a "Databases" group: default write mode for new databases (Confirm writes), key separator default (`:`), page size (500), "Keep console history" (on).

## 6. Tests

Follow `replica/testing/test-plan.md` layers.

- **Servers** (`tests/servers/compose.yml`): `redis:7` single with an ACL user and a password; one with TLS (self-signed CA in fixtures); a 6-node cluster; one primary, two replicas and three sentinels; all reachable **only** from the existing sshd container's network so the "through a host" path is the one tested, plus one published port for the direct path. Seed script with every type, a binary key, a 2 MB string, a 50,000-member sorted set and 20,000 `session:*` keys.
- **Rust unit:** the gate table (every row of §4, table-driven, including unknown commands, `MULTI`, case and whitespace variants, `CONFIG` subcommands); the console line parser (quotes, escapes, binary); CRC16 and hash tags against known vectors; `MOVED`/`ASK` handling with a fake node; prepare/commit (double commit, expiry, wrong connection, wrong typed name); history filter.
- **Rust integration** (ignored unless the servers are up, like the sshd tests): connect direct, through a host, through a jump chain, TLS good and untrusted, wrong password, replica; scan paging; each type's read and write; bulk delete with stop; cluster scan across nodes and a redirect during a resharding; sentinel failover.
- **Fake backend:** a Redis fake with a small in-memory keyspace and the gate's decisions scripted, so browser tests cover S26 to S33 without Docker.
- **Browser (e2e):** F13 to F19; read-only hides controls and refuses in the console; confirm shows before/after and the exact command; typed confirm stays disabled until the name matches; Cancel is focused; lock closes the tab's connections; a production tab shows the line and chip.
- **Visual:** baselines for the seven boards' states in dark, and S28 in light. **Accessibility:** axe on each view; tree and table keyboard paths.
- **Walk:** a new chapter in `pnpm walk` (and `walk:app` against the Docker servers).
- **Performance:** scanning 1,000,000 keys stays responsive and under the memory budget in `test-plan.md`; a 50,000-row value scrolls smoothly; a Pub/Sub flood of 10,000 messages a second does not freeze the tab (backpressure).
- **Hand checks:** add section I to `replica/testing/hand-checks.md`: a real staging database through the bastion, a real cluster if the team has one, the amber line on production, mode change asking for the master password, and trying to break the gate from the console.
- **features.csv:** add one row per feature below with `original` = `no` (these are our own additions, not part of the original app), following the file's rules (`pnpm check:features`).

## 7. Phases

Each is one `go`. Read this spec and the boards named before each.

- **R1 Foundations (S26, S27, S33 connecting; F13):** migration, crates, `redis/` module with the transport for direct and through-a-host (single server), TLS, `redis_db_*`, `redis_open`/`close`/`select`, the Databases section, the sheet with Test connection, the Redis tab shell with its toolbar and connection states, host menu and palette entries, lock. Docker single server. Decision to bring back if it comes up: the `redis` crate cannot take our stream (then the small RESP codec).
- **R2 Browser, read side (S28, S29; F14):** scan, tree, type filter, key info, every type's viewer with paging, big-value preview, binary handling, JSON view. No writes yet: build the tab as if Read-only.
- **R3 The write gate and editors (S33, S28, S29; F15):** `gate.rs`, prepare/commit, modes and `redis_set_mode`, the two dialogs, read-only banner, every editor action, new key, rename, expiry, delete. The gate's unit table is part of this phase, not later.
- **R4 Console and Server (S30, S31; F16):** line parser, completion from the command table, reply formatting, encrypted history with the filter, heavy and refused classes in the console; info tiles, slow commands, keys by database.
- **R5 Pub/Sub and bulk actions (S32, S33; F17, F18):** the listener connection and channel, pause/filter/clear, publish through the gate; bulk preview, run, stop.
- **R6 Cluster and Sentinel (S27, S31; F19):** slot map, redirects, merged scan, per-slot bulk batches, sentinel lookup and re-lookup, Nodes panel, the Docker cluster and sentinel sets.
- **R7 Hardening and hand checks:** performance tests, walk chapter, visual baselines, section I of the hand checks, `features.csv`, Settings group, a short "Databases" part in the README/onboarding. Ends `[h]` until Mehrdad reports back.

## 8. Not in this version

Live command tracing (`MONITOR`), key-space memory analysis, editing binary values, import/export of keys, Streams consumer-group management beyond listing, module data types other than JSON (search, time series, probabilistic), RDB analysis, SSH-less cloud provider sign-ins, sharing databases between teammates (that waits for sync).

#!/usr/bin/env bash
# Throwaway servers for Dalan's real-server tests and for testing by hand. Never point this at a real box.
# The containers are defined in tests/servers/compose.yml; this script makes the keys, starts them, tweaks sshd,
# and writes .test-servers/servers.json, which the Rust tests (cargo test -- --ignored) and the real-app tests read.
#
#   scripts/test-servers.sh up [--extra]       start the servers (--extra adds mosh, which needs the mosh client on this computer)
#   scripts/test-servers.sh down               stop and remove them
#   scripts/test-servers.sh status             what is running, and each SSH server's host key fingerprints
#   scripts/test-servers.sh restart <name>     restart one server (for the "connection dropped" tests)
#   scripts/test-servers.sh rotate <name> [--type ecdsa]
#                                              give a server new host keys (F12); --type ecdsa makes it present an ECDSA key only
#   scripts/test-servers.sh otp                print the current "Verification code" for the kbd server
#   scripts/test-servers.sh rekey <a|b|c> [new|only-ed25519|only-ecdsa|only-rsa|restore]    (older name for rotate, with more modes)
#   scripts/test-servers.sh kex-break <a|b|c> | kex-restore <a|b|c>   (make the handshake fail with an old cipher)
#
# Servers (127.0.0.1 unless said otherwise; "net" = the name inside the Docker network):
#   a       2222  user test / password test     password sign-in; the bastion (net: bastion); forwarding on, GatewayPorts, AcceptEnv
#   b       2223  user keyuser, key only        keys in .test-servers/ (plain and passphrase-protected)
#   c       -     user test / password test     only reachable from a, as inner:2222 (jump chain)
#   kbd     2224  user test / test + code       keyboard-interactive: Password, then Verification code (see `otp`)
#   slow    2225  forwards to a, 800 ms delay   slow connect, the step log
#   oldkex  2226  user test / password test     offers only an old key exchange: "couldn't agree on encryption"
#   rot     2228  user test / password test     the server the tests rotate host keys on
#   web     8088  nginx                         something to reach through a tunnel (net: web:80)
#   squid   3128  HTTP CONNECT proxy            (net: squid:3128)
#   socks   1080  SOCKS5 proxy, no sign-in      (net: socks:1080)
#   telnet  2323  busybox shell, no sign-in     Telnet
#   mosh    2227  user test / password test     only with --extra
set -euo pipefail

cd "$(dirname "$0")/.."
COMPOSE=(docker compose -f tests/servers/compose.yml)
KEYS_DIR=".test-servers"
# (macOS ships bash 3.2, which has no associative arrays, hence functions.)
name_of() {
  case "$1" in
    a) echo dalan-test-ssh ;; b) echo dalan-test-key ;; c) echo dalan-test-inner ;; kbd) echo dalan-test-kbd ;;
    slow) echo dalan-test-slow ;; oldkex) echo dalan-test-oldkex ;; web) echo dalan-test-web ;; squid) echo dalan-test-squid ;;
    socks) echo dalan-test-socks ;; telnet) echo dalan-test-telnet ;; mosh) echo dalan-test-mosh ;; rot) echo dalan-test-rot ;; twin) echo dalan-test-twin ;; *) return 1 ;;
  esac
}
ALL="a b c kbd slow oldkex rot twin web squid socks telnet"
SSH_SERVERS="a b c kbd oldkex rot twin mosh"

need() { command -v "$1" >/dev/null || { echo "Missing: $1" >&2; exit 1; }; }
need docker
need python3
docker info >/dev/null 2>&1 || { echo "Docker isn't running. Start Docker Desktop and try again." >&2; exit 1; }

c_of() { name_of "${1:-}" || { echo "Pick a server: a, b, c, kbd, slow, oldkex, rot, web, squid, socks, telnet or mosh." >&2; exit 1; }; }
running() { [ "$(docker inspect -f '{{.State.Running}}' "$1" 2>/dev/null || true)" = "true" ]; }
is_linuxserver() { case "$1" in a | b | c) return 0 ;; *) return 1 ;; esac; }

wait_sshd() { # container
  for _ in $(seq 1 90); do
    # linuxserver: the config exists and the listener runs. Debian variants: the listener runs.
    if docker exec "$1" sh -c '(test -f /config/sshd/sshd_config && ps aux | grep -q "[s]shd.pam -D") || ps aux 2>/dev/null | grep -q "[s]shd -D"' >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "Warning: $1 didn't report ready in 90 s (check: docker logs $1)" >&2
}

set_conf() { # container 'sed expression' 'line to add if the setting is missing' 'setting name'
  local c="$1" expr="$2" add="$3" key="$4"
  docker exec "$c" sh -c "grep -qE '^#?$key' /config/sshd/sshd_config && sed -i -E '$expr' /config/sshd/sshd_config || echo '$add' >> /config/sshd/sshd_config"
}

make_keys() {
  mkdir -p "$KEYS_DIR"
  [ -f "$KEYS_DIR/id_ed25519" ] || ssh-keygen -q -t ed25519 -N "" -C "dalan-test" -f "$KEYS_DIR/id_ed25519"
  [ -f "$KEYS_DIR/id_ed25519_passphrase" ] || ssh-keygen -q -t ed25519 -N "passphrase" -C "dalan-test-passphrase" -f "$KEYS_DIR/id_ed25519_passphrase"
  # Server b accepts both keys.
  cat "$KEYS_DIR/id_ed25519.pub" "$KEYS_DIR/id_ed25519_passphrase.pub" > "$KEYS_DIR/authorized.pub"
  # The kbd server's one-time-code secret, in the format the PAM module reads.
  if [ ! -f "$KEYS_DIR/otp-secret" ]; then
    python3 - "$KEYS_DIR/otp-secret" <<'PY'
import base64, os, sys
secret = base64.b32encode(os.urandom(10)).decode()
open(sys.argv[1], "w").write(f'{secret}\n" WINDOW_SIZE 3\n" TOTP_AUTH\n')
PY
    chmod 644 "$KEYS_DIR/otp-secret"
  fi
}

otp() { # the current code, from the same secret the server holds
  python3 - "$KEYS_DIR/otp-secret" <<'PY'
import base64, hashlib, hmac, struct, sys, time
secret = open(sys.argv[1]).readline().strip()
key = base64.b32decode(secret + "=" * (-len(secret) % 8))
digest = hmac.new(key, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
o = digest[-1] & 15
print(f"{(struct.unpack('>I', digest[o:o + 4])[0] & 0x7fffffff) % 10**6:06d}")
PY
}

fingerprint() { # container -> ed25519 SHA256 fingerprint of its host key ("" if none)
  docker exec "$1" sh -c 'for f in /config/ssh_host_keys/ssh_host_ed25519_key.pub /etc/ssh/ssh_host_ed25519_key.pub; do [ -f "$f" ] && ssh-keygen -lf "$f" && break; done' 2>/dev/null | awk '{print $2}' | head -1
}

write_servers_json() {
  python3 - "$KEYS_DIR/servers.json" "$KEYS_DIR/otp-secret" <<PY
import json, sys
fp = {
$(for k in $SSH_SERVERS; do c="$(name_of $k)"; if running "$c"; then echo "    \"$k\": \"$(fingerprint "$c")\","; fi; done)
}
def s(name, port, **kw):
    return dict(name=name, host="127.0.0.1", port=port, container=kw.pop("container"), **kw)
servers = {
    "a": s("a", 2222, container="dalan-test-ssh", user="test", password="test", net_host="bastion", net_port=2222, fingerprint=fp.get("a")),
    "b": s("b", 2223, container="dalan-test-key", user="keyuser", key=".test-servers/id_ed25519", key_with_passphrase=".test-servers/id_ed25519_passphrase", passphrase="passphrase", net_host="keyhost", net_port=2222, fingerprint=fp.get("b")),
    "c": s("c", None, container="dalan-test-inner", user="test", password="test", net_host="inner", net_port=2222, fingerprint=fp.get("c")),
    "kbd": s("kbd", 2224, container="dalan-test-kbd", user="test", password="test", code_command="scripts/test-servers.sh otp", net_host="kbd", net_port=22, fingerprint=fp.get("kbd")),
    "slow": s("slow", 2225, container="dalan-test-slow", user="test", password="test", delay_ms=800, net_host="slow", net_port=2225),
    "oldkex": s("oldkex", 2226, container="dalan-test-oldkex", user="test", password="test", net_host="oldkex", net_port=22, fingerprint=fp.get("oldkex")),
    "rot": s("rot", 2228, container="dalan-test-rot", user="test", password="test", net_host="rot", net_port=22, fingerprint=fp.get("rot")),
    "twin": s("twin", 2229, container="dalan-test-twin", user="test", password="test", net_host="twin", net_port=22, fingerprint=fp.get("twin")),
    "web": s("web", 8088, container="dalan-test-web", net_host="web", net_port=80),
    "squid": s("squid", 3128, container="dalan-test-squid", net_host="squid", net_port=3128),
    "socks": s("socks", 1080, container="dalan-test-socks", net_host="socks", net_port=1080),
    "telnet": s("telnet", 2323, container="dalan-test-telnet", net_host="telnet", net_port=23),
}
if fp.get("mosh"):
    servers["mosh"] = s("mosh", 2227, container="dalan-test-mosh", user="test", password="test", net_host="mosh", net_port=22, fingerprint=fp["mosh"])
json.dump(servers, open(sys.argv[1], "w"), indent=2)
PY
}

configure_a() {
  # a is the jump host and the tunnel server: forwarding on, remote forwards on any address, and environment variables accepted.
  local c; c="$(name_of a)"
  set_conf "$c" 's/^#?AllowTcpForwarding.*/AllowTcpForwarding yes/' 'AllowTcpForwarding yes' 'AllowTcpForwarding'
  set_conf "$c" 's/^#?GatewayPorts.*/GatewayPorts clientspecified/' 'GatewayPorts clientspecified' 'GatewayPorts'
  docker exec "$c" sh -c "grep -q '^AcceptEnv LANG LC_\\* DALAN_\\*' /config/sshd/sshd_config || echo 'AcceptEnv LANG LC_* DALAN_*' >> /config/sshd/sshd_config"
  docker restart -t 2 "$c" >/dev/null
  wait_sshd "$c"
}

up() {
  make_keys
  local extra=()
  [ "${1:-}" = "--extra" ] && extra=(--profile extra)
  # Containers and the network from an older version of this script (made with `docker run`) would clash on their names.
  for k in a b c; do
    local old; old="$(name_of $k)"
    if docker inspect "$old" >/dev/null 2>&1 && [ -z "$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project"}}' "$old" 2>/dev/null)" ]; then docker rm -f "$old" >/dev/null; fi
  done
  if docker network inspect dalan-test-net >/dev/null 2>&1 && [ -z "$(docker network inspect -f '{{index .Labels "com.docker.compose.network"}}' dalan-test-net 2>/dev/null)" ]; then
    docker network rm dalan-test-net >/dev/null 2>&1 || true
  fi
  ${COMPOSE[@]+"${COMPOSE[@]}"} ${extra[@]+"${extra[@]}"} up -d --build >/dev/null
  for k in a b c kbd oldkex rot twin; do wait_sshd "$(name_of $k)"; done
  [ "${1:-}" = "--extra" ] && wait_sshd "$(name_of mosh)"
  configure_a
  write_servers_json
  echo
  echo "Servers are up. Details for the tests are in $KEYS_DIR/servers.json. To add them by hand in Dalan:"
  echo "  password   127.0.0.1  port 2222  user test     password test"
  echo "  key-only   127.0.0.1  port 2223  user keyuser  (import $PWD/$KEYS_DIR/id_ed25519 or id_ed25519_passphrase, passphrase: passphrase)"
  echo "  inner      inner      port 2222  user test     password test   -> Jump hosts: Custom, hop = the password host above"
  echo "  2FA        127.0.0.1  port 2224  user test     password test, then the code from: scripts/test-servers.sh otp"
  echo "  slow       127.0.0.1  port 2225  user test     password test   (every step takes about a second)"
  echo "  old kex    127.0.0.1  port 2226  user test     (fails: couldn't agree on encryption)"
  echo "  telnet     127.0.0.1  port 2323  (Telnet, no sign-in)    proxies: HTTP 127.0.0.1:3128, SOCKS5 127.0.0.1:1080    tunnel target: http://127.0.0.1:8088"
  echo "Also try: 127.0.0.1 port 2299 (nothing listens: unreachable), and a wrong password."
}

down() {
  ${COMPOSE[@]+"${COMPOSE[@]}"} --profile extra down --remove-orphans -t 2 >/dev/null 2>&1 || true
  # Anything left from the older `docker run` version of this script.
  for k in $ALL mosh; do docker rm -f "$(name_of $k)" >/dev/null 2>&1 || true; done
  docker network rm dalan-test-net >/dev/null 2>&1 || true
  echo "Stopped and removed the test servers (the keys in $KEYS_DIR are kept)."
}

status() {
  for k in $ALL mosh; do
    local c; c="$(name_of $k)"
    if running "$c"; then
      echo "[$k] $c: running"
      case " $SSH_SERVERS " in
        *" $k "*)
          docker exec "$c" sh -c 'for f in /config/ssh_host_keys/*.pub /etc/ssh/ssh_host_*_key.pub; do [ -f "$f" ] && ssh-keygen -lf "$f" 2>/dev/null; done' || true
          docker exec "$c" sh -c "grep -hE '^(HostKeyAlgorithms|AllowTcpForwarding|GatewayPorts|Ciphers|KexAlgorithms)' /config/sshd/sshd_config /etc/ssh/sshd_config" 2>/dev/null | sed 's/^/    /' || true ;;
      esac
    else
      echo "[$k] $c: not running"
    fi
  done
}

restart() { # name
  local c; c="$(c_of "${1:-}")"
  running "$c" || { echo "$c isn't running. Run: $0 up" >&2; exit 1; }
  docker restart -t 2 "$c" >/dev/null
  case " $SSH_SERVERS " in *" $1 "*) wait_sshd "$c" ;; esac
  echo "$c restarted."
}

rotate() { # name [--type ecdsa]
  local k="${1:-}"; local c; c="$(c_of "$k")"
  running "$c" || { echo "$c isn't running. Run: $0 up" >&2; exit 1; }
  if is_linuxserver "$k"; then
    if [ "${2:-}" = "--type" ] && [ "${3:-}" = "ecdsa" ]; then rekey "$k" new; rekey "$k" only-ecdsa; else rekey "$k" new; fi
  else
    docker exec "$c" sh -c 'rm -f /etc/ssh/ssh_host_*'
    docker restart -t 2 "$c" >/dev/null; wait_sshd "$c"
  fi
  write_servers_json
}

rekey() { # a|b|c  mode
  local c; c="$(c_of "${1:-}")"; local mode="${2:-new}"
  running "$c" || { echo "$c isn't running. Run: $0 up" >&2; exit 1; }
  case "$mode" in
    new) # replace every host key with a fresh one (the image only makes keys on its very first start)
      docker exec "$c" sh -c 'cd /config/ssh_host_keys && rm -f ssh_host_* \
        && ssh-keygen -q -N "" -t ed25519 -f ssh_host_ed25519_key \
        && ssh-keygen -q -N "" -t ecdsa -b 256 -f ssh_host_ecdsa_key \
        && ssh-keygen -q -N "" -t rsa -b 3072 -f ssh_host_rsa_key \
        && chmod 600 ssh_host_*_key && chmod 644 ssh_host_*.pub'
      docker restart -t 2 "$c" >/dev/null ;;
    only-ed25519|only-ecdsa|only-rsa)
      # The image passes every host key to sshd on the command line, so limit what sshd *offers* instead:
      # HostKeyAlgorithms makes the server present just that key type.
      local algs
      case "$mode" in
        only-ed25519) algs="ssh-ed25519" ;;
        only-ecdsa) algs="ecdsa-sha2-nistp256" ;;
        only-rsa) algs="rsa-sha2-512,rsa-sha2-256" ;;
      esac
      set_conf "$c" "s/^#?HostKeyAlgorithms.*/HostKeyAlgorithms $algs/" "HostKeyAlgorithms $algs" 'HostKeyAlgorithms'
      docker restart -t 2 "$c" >/dev/null ;;
    restore)
      docker exec "$c" sh -c "sed -i -E '/^HostKeyAlgorithms /d' /config/sshd/sshd_config"
      docker restart -t 2 "$c" >/dev/null ;;
    *) echo "Unknown mode: $mode" >&2; exit 1 ;;
  esac
  wait_sshd "$c"
  local port; port="$(docker port "$c" 2222 2>/dev/null | head -1 | sed 's/.*://')"
  if [ -n "$port" ] && command -v ssh-keyscan >/dev/null; then
    echo "Host key now presented by $c (port $port):"
    ssh-keyscan -p "$port" 127.0.0.1 2>/dev/null | ssh-keygen -lf - 2>/dev/null | sed 's/^/    /'
  else
    echo "Done. $c restarted (it has no published port, so no fingerprint is shown)."
  fi
}

kex_break() { # a|b|c  (optional) only offer a cipher Dalan doesn't speak, so the handshake fails
  local c; c="$(c_of "${1:-}")"
  set_conf "$c" 's/^#?Ciphers.*/Ciphers aes128-cbc/' 'Ciphers aes128-cbc' 'Ciphers'
  docker restart -t 2 "$c" >/dev/null; wait_sshd "$c"
  echo "$c now offers only aes128-cbc. If Dalan still connects, it supports that cipher; skip the KEX step."
}
kex_restore() {
  local c; c="$(c_of "${1:-}")"
  docker exec "$c" sh -c "sed -i -E '/^Ciphers aes128-cbc/d' /config/sshd/sshd_config"
  docker restart -t 2 "$c" >/dev/null; wait_sshd "$c"; echo "$c is back to its default ciphers."
}

case "${1:-}" in
  up) up "${2:-}" ;;
  down) down ;;
  status) status ;;
  restart) restart "${2:-}" ;;
  rotate) rotate "${2:-}" "${3:-}" "${4:-}" ;;
  otp) make_keys; otp ;;
  rekey) rekey "${2:-}" "${3:-new}" ;;
  kex-break) kex_break "${2:-}" ;;
  kex-restore) kex_restore "${2:-}" ;;
  *) sed -n '2,29p' "$0"; exit 1 ;;
esac

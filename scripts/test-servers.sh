#!/usr/bin/env bash
# Throwaway SSH servers for testing Dalan by hand (replica/smoke-test.md). Never point this at a real box.
#
#   scripts/test-servers.sh up                 start the three servers
#   scripts/test-servers.sh down               stop and remove them
#   scripts/test-servers.sh status             what is running, and each server's host key fingerprints
#   scripts/test-servers.sh rekey <a|b|c> [new|only-ed25519|only-ecdsa|only-rsa|restore]
#                                              change what host key a server presents (F12)
#   scripts/test-servers.sh kex-break <a|b|c> | kex-restore <a|b|c>   (optional: make the handshake fail)
#
# Servers (all use the linuxserver/openssh-server image; sshd listens on 2222 inside):
#   a  "bastion"  127.0.0.1:2222  user test / password test      password sign-in, forwarding on (jump host, tunnels, SFTP)
#   b  "key"      127.0.0.1:2223  user keyuser, key only         keys in .test-servers/ (plain and passphrase-protected)
#   c  "inner"    not published   user test / password test      only reachable as inner:2222 through a (jump chain)
set -euo pipefail

cd "$(dirname "$0")/.."
IMAGE="lscr.io/linuxserver/openssh-server"
NET="dalan-test-net"
KEYS_DIR=".test-servers"
# (macOS ships bash 3.2, which has no associative arrays, hence a function.)
name_of() { case "$1" in a) echo dalan-test-ssh ;; b) echo dalan-test-key ;; c) echo dalan-test-inner ;; *) return 1 ;; esac; }

need() { command -v "$1" >/dev/null || { echo "Missing: $1" >&2; exit 1; }; }
need docker
docker info >/dev/null 2>&1 || { echo "Docker isn't running. Start Docker Desktop and try again." >&2; exit 1; }

c_of() { name_of "${1:-}" || { echo "Pick a server: a, b or c." >&2; exit 1; }; }
running() { [ "$(docker inspect -f '{{.State.Running}}' "$1" 2>/dev/null || true)" = "true" ]; }

wait_sshd() { # container
  for _ in $(seq 1 60); do
    # Ready = the config exists and the sshd listener process is running.
    if docker exec "$1" sh -c 'test -f /config/sshd/sshd_config && ps aux | grep -q "[s]shd.pam -D"' >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "Warning: $1 didn't report ready in 60 s (check: docker logs $1)" >&2
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
}

up() {
  make_keys
  docker network inspect "$NET" >/dev/null 2>&1 || docker network create "$NET" >/dev/null
  if ! running "$(name_of a)"; then
    docker rm -f "$(name_of a)" >/dev/null 2>&1 || true
    docker run -d --name "$(name_of a)" --network "$NET" --network-alias bastion -p 2222:2222 \
      -e PUID=1000 -e PGID=1000 -e PASSWORD_ACCESS=true -e USER_NAME=test -e USER_PASSWORD=test "$IMAGE" >/dev/null
  fi
  if ! running "$(name_of b)"; then
    docker rm -f "$(name_of b)" >/dev/null 2>&1 || true
    docker run -d --name "$(name_of b)" --network "$NET" --network-alias keyhost -p 2223:2222 \
      -v "$PWD/$KEYS_DIR:/keys:ro" \
      -e PUID=1000 -e PGID=1000 -e PASSWORD_ACCESS=false -e USER_NAME=keyuser -e PUBLIC_KEY_FILE=/keys/authorized.pub "$IMAGE" >/dev/null
  fi
  if ! running "$(name_of c)"; then
    docker rm -f "$(name_of c)" >/dev/null 2>&1 || true
    docker run -d --name "$(name_of c)" --network "$NET" --network-alias inner \
      -e PUID=1000 -e PGID=1000 -e PASSWORD_ACCESS=true -e USER_NAME=test -e USER_PASSWORD=test "$IMAGE" >/dev/null
  fi
  for k in a b c; do wait_sshd "$(name_of $k)"; done
  # a is the jump host and the tunnel server: forwarding on, remote forwards allowed on all interfaces.
  set_conf "$(name_of a)" 's/^#?AllowTcpForwarding.*/AllowTcpForwarding yes/' 'AllowTcpForwarding yes' 'AllowTcpForwarding'
  set_conf "$(name_of a)" 's/^#?GatewayPorts.*/GatewayPorts clientspecified/' 'GatewayPorts clientspecified' 'GatewayPorts'
  docker restart -t 2 "$(name_of a)" >/dev/null
  wait_sshd "$(name_of a)"
  echo
  echo "Servers are up. Add these hosts in Dalan:"
  echo "  password   127.0.0.1  port 2222  user test     password test"
  echo "  key-only   127.0.0.1  port 2223  user keyuser  (import $PWD/$KEYS_DIR/id_ed25519 or id_ed25519_passphrase, passphrase: passphrase)"
  echo "  inner      inner      port 2222  user test     password test   -> Jump hosts: Custom, hop = the password host above"
  echo "Also try: 127.0.0.1 port 2299 (nothing listens: unreachable), and a wrong password."
}

down() {
  for k in a b c; do docker rm -f "$(name_of $k)" >/dev/null 2>&1 || true; done
  docker network rm "$NET" >/dev/null 2>&1 || true
  echo "Stopped and removed the test servers (the keys in $KEYS_DIR are kept)."
}

status() {
  for k in a b c; do
    local c="$(name_of $k)"
    if running "$c"; then
      echo "[$k] $c: running"
      docker exec "$c" sh -c 'for f in /config/ssh_host_keys/*.pub; do ssh-keygen -lf "$f" 2>/dev/null; done' || true
      docker exec "$c" sh -c "grep -E '^(HostKeyAlgorithms|AllowTcpForwarding|GatewayPorts|Ciphers)' /config/sshd/sshd_config" 2>/dev/null | sed 's/^/    /' || true
    else
      echo "[$k] $c: not running"
    fi
  done
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
  up) up ;;
  down) down ;;
  status) status ;;
  rekey) rekey "${2:-}" "${3:-new}" ;;
  kex-break) kex_break "${2:-}" ;;
  kex-restore) kex_restore "${2:-}" ;;
  *) sed -n '2,13p' "$0"; exit 1 ;;
esac

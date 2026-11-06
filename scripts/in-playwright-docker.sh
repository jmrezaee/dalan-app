#!/usr/bin/env bash
# Runs a command inside the pinned Playwright image (Linux), so screenshots match on every machine.
#   scripts/in-playwright-docker.sh pnpm exec playwright test --project=visual
#   scripts/in-playwright-docker.sh pnpm exec playwright test --project=visual --update-snapshots
# The repository is mounted at /work; node_modules and the pnpm store live in Docker volumes, so the Mac's own
# node_modules are untouched and the second run is fast.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(node -p "require('./node_modules/@playwright/test/package.json').version")"
IMAGE="mcr.microsoft.com/playwright:v${VERSION}-noble"
docker info >/dev/null 2>&1 || { echo "Docker isn't running. Start Docker Desktop and try again." >&2; exit 1; }
exec docker run --rm --ipc=host \
  -v "$PWD":/work -w /work \
  -v dalan-pw-node-modules:/work/node_modules \
  -v dalan-pw-pnpm-store:/root/.local/share/pnpm/store \
  -e CI=1 -e DALAN_IN_DOCKER=1 \
  "$IMAGE" bash -lc '
    set -e
    corepack enable >/dev/null 2>&1 || npm i -g pnpm@10.30.0 >/dev/null
    pnpm install --frozen-lockfile --ignore-scripts >/dev/null
    node scripts/sync-tokens.mjs
    pnpm exec nuxt prepare >/dev/null
    "$@"
  ' bash "$@"

#!/usr/bin/env bash
# Runs inside the image: builds the real app (with the test hooks), starts a virtual screen and a recorder, and runs
# WebdriverIO against the binary. Everything it writes goes to test-results/app/.
set -euo pipefail
cd /work
OUT=test-results/app
mkdir -p "$OUT"

echo "== dependencies"
pnpm install --frozen-lockfile --ignore-scripts >/dev/null
node scripts/sync-tokens.mjs
pnpm exec nuxt prepare >/dev/null

echo "== building the app (debug, with test hooks)"
DALAN_TEST=1 DALAN_APP_TEST=1 pnpm exec tauri build --debug --no-bundle

echo "== starting the screen and the recorder"
export DISPLAY=:99
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp &
sleep 2
ffmpeg -loglevel error -y -f x11grab -video_size 1280x800 -framerate 10 -i :99 -pix_fmt yuv420p "$OUT/run.mp4" &
FFMPEG=$!
export XDG_DATA_HOME=/tmp/dalan-xdg WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 DALAN_BIN=/work/src-tauri/target/debug/dalan-app

echo "== running the tests"
set +e
dbus-run-session -- pnpm exec wdio run tests/app/wdio.conf.ts "$@"
STATUS=$?
set -e
kill -INT "$FFMPEG" 2>/dev/null || true
wait "$FFMPEG" 2>/dev/null || true
exit $STATUS

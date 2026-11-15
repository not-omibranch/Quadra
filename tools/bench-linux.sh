#!/bin/sh
# Connection benchmark on Linux, headless: runs a debug build of the app under Xvfb with a
# bench job (see `bench` in src-tauri/src/lib.rs) and its own data folder, so a real Quadra on
# the same machine is not touched. The app exits by itself when the job is done.
#
#   tools/bench-linux.sh <job.json> <work dir> [timeout seconds]
#
# Job example (ports mode keeps the system untouched):
#   {"link": "vless://...", "settings": {"mode": "ports", "socks_port": 21808, "http_port": 21809, "core": "auto"},
#    "cycles": 20, "pause_ms": 500, "download_url": "https://speed.cloudflare.com/__down?bytes=5000000",
#    "soak_secs": 0}
# "core": "xray" forces Xray; "xhttp_via_xray": false sends XHTTP servers to sing-box; "auto": true
# connects to "Авто, самый быстрый" over every server in "link".
# Results: <work dir>/dev.quadra.client/bench-result.json and, with soak_secs, bench-soak.json.
# Build first: npx tauri build --debug --no-bundle (and node tools/fetch-singbox.mjs, fetch-xray.mjs).
set -e
job=$1; work=$2; limit=${3:-900}
[ -f "$job" ] && [ -n "$work" ] || { echo "usage: $0 <job.json> <work dir> [timeout]"; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
app=$root/src-tauri/target/debug/quadra
[ -x "$app" ] || { echo "no debug build at $app"; exit 2; }
mkdir -p "$work/dev.quadra.client"
work=$(cd "$work" && pwd)
cp "$job" "$work/dev.quadra.client/bench.json"
cd "$work"
XDG_DATA_HOME=$work XDG_CONFIG_HOME=$work/config XDG_CACHE_HOME=$work/cache HOME=$work WEBKIT_DISABLE_COMPOSITING_MODE=1 \
  timeout "$limit" xvfb-run -a "$app" > "$work/app.out" 2>&1 || echo "app exit code $?"
cat "$work/dev.quadra.client/bench-result.json" 2>/dev/null | head -c 2000; echo
[ -f "$work/dev.quadra.client/bench-soak.json" ] && cat "$work/dev.quadra.client/bench-soak.json"
exit 0

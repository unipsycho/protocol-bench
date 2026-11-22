#!/usr/bin/env bash
# One-command dev restart: stop old service + UI, rebuild, refresh modules/,
# start the service, then start the UI. Closing the UI stops the service.
#
#   scripts/dev.sh                      # debug build
#   scripts/dev.sh --release            # release build
#   scripts/dev.sh --project FILE       # service opens FILE (else PB_PROJECT / default)
#   scripts/dev.sh --no-ui              # service only (foreground)
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$PWD

profile=debug; cargo_flag=""; project="${PB_PROJECT:-}"; ui=1
while [ $# -gt 0 ]; do
  case "$1" in
    --release) profile=release; cargo_flag="--release" ;;
    --project) shift; project="$(realpath "$1")" ;;
    --no-ui) ui=0 ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

PACKS=(pb-mqtt pb-opcua pb-streams pb-http pb-modbus pb-files pb-snmp pb-las)

echo "==> stopping old service / UI"
pkill -x protocol-bench 2>/dev/null || true
pkill -x pb-service 2>/dev/null || true
for _ in $(seq 20); do
  pgrep -x pb-service >/dev/null || pgrep -x protocol-bench >/dev/null || break
  sleep 0.25
done

echo "==> building ($profile)"
args=(); for p in "${PACKS[@]}"; do args+=(-p "$p"); done
cargo build $cargo_flag "${args[@]}" -p pb-service -p pb-ui

echo "==> refreshing modules/"
mkdir -p modules
rm -f modules/libpb_*.so modules/pb_*.dll
cp "target/$profile"/libpb_*.so modules/ 2>/dev/null || cp "target/$profile"/pb_*.dll modules/

[ -n "$project" ] && export PB_PROJECT="$project"
export PB_MODULES="$ROOT/modules"

if [ "$ui" -eq 0 ]; then
  exec "target/$profile/pb-service"
fi

echo "==> starting service"
cfg="${XDG_CONFIG_HOME:-$HOME/.config}/protocolbench"
rm -f "$cfg/ipc.json"            # so we wait for THIS service's port, not a stale one
"target/$profile/pb-service" &
svc=$!
trap 'kill "$svc" 2>/dev/null || true; wait "$svc" 2>/dev/null || true' EXIT INT TERM

for _ in $(seq 40); do
  [ -s "$cfg/ipc.json" ] && break
  kill -0 "$svc" 2>/dev/null || { echo "service exited early" >&2; exit 1; }
  sleep 0.25
done
[ -s "$cfg/ipc.json" ] || { echo "service never wrote $cfg/ipc.json" >&2; exit 1; }

echo "==> starting UI (close it to stop the service)"
"target/$profile/protocol-bench"

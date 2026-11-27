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

PACKS=(pb-mqtt pb-opcua pb-streams pb-http pb-modbus pb-files pb-snmp pb-las pb-wits pb-websocket pb-sparkplug pb-dnp3 pb-can pb-gen pb-dashboard)

echo "==> stopping old service / UI"
# pgrep hangs on a process stuck in the kernel, so scan /proc directly.
pids_named() {  # print PIDs whose comm is $1
  local p c
  for p in /proc/[0-9]*; do
    { read -r c < "$p/comm"; } 2>/dev/null || continue
    [ "$c" = "$1" ] && echo "${p#/proc/}"
  done
}
stale() { [ -n "$(pids_named pb-service)$(pids_named protocol-bench)" ]; }
for pid in $(pids_named protocol-bench) $(pids_named pb-service); do kill "$pid" 2>/dev/null || true; done
for _ in $(seq 20); do stale || break; sleep 0.25; done
if stale; then
  for pid in $(pids_named protocol-bench) $(pids_named pb-service); do kill -KILL "$pid" 2>/dev/null || true; done
  for _ in $(seq 20); do stale || break; sleep 0.25; done
fi
if stale; then
  # A process in uninterruptible sleep (state D, usually a driver call) ignores
  # even SIGKILL until the kernel releases it. Starting a new service now would
  # fight it for ports, so stop here and say so.
  for name in pb-service protocol-bench; do
    for pid in $(pids_named "$name"); do
      st=$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || echo "?")
      echo "error: $name (pid $pid) is still running, state $st" >&2
      [ "$st" = "D" ] && echo "       state D = stuck in the kernel; it cannot be killed until it returns. Try again later, or reboot." >&2
    done
  done
  exit 1
fi

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

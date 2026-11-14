#!/usr/bin/env bash
# Build pb-mqtt into modules/, then subscribe→point and publish against a local broker.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== build pb-mqtt =="
cargo build -p pb-mqtt -p pb-engine -p pb-service

mkdir -p modules
SO="target/debug/libpb_mqtt.so"
if [[ ! -f "$SO" ]]; then
  echo "missing $SO" >&2
  exit 1
fi
cp -f "$SO" modules/
echo "installed modules/libpb_mqtt.so"

if ! command -v mosquitto >/dev/null 2>&1 || ! command -v mosquitto_pub >/dev/null 2>&1; then
  cat <<EOF
Mosquitto not found. Pack is in modules/. To smoke manually:
  1. Start an anonymous broker (e.g. mosquitto -p 18884 -v with allow_anonymous true)
  2. PB_SMOKE_HOST=127.0.0.1 PB_SMOKE_PORT=18884 cargo test -p pb-mqtt --test smoke -- --nocapture
  3. Or: PB_MODULES="$ROOT/modules" DISPLAY=:0 cargo run -p pb-ui
     then Connect / Subscribe / Publish in the MQTT panel
EOF
  exit 0
fi

# Always use a dedicated anonymous listener so a plant Mosquitto on 1883 cannot block smoke.
USE_PORT="${PB_SMOKE_PORT:-18884}"
CONF="$(mktemp)"
cat >"$CONF" <<EOF
listener ${USE_PORT} 127.0.0.1
allow_anonymous true
EOF
mosquitto -c "$CONF" -d
sleep 0.5
echo "started temporary mosquitto on 127.0.0.1:${USE_PORT}"

cleanup() {
  # Kill listeners on our smoke port (mosquitto -d forks).
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "${USE_PORT}/tcp" 2>/dev/null || true
  else
    pkill -f "mosquitto -c ${CONF}" 2>/dev/null || true
  fi
  rm -f "$CONF"
}
trap cleanup EXIT

echo "== smoke =="
PB_MODULES="$ROOT/modules" PB_SMOKE_HOST=127.0.0.1 PB_SMOKE_PORT="$USE_PORT" \
  cargo test -p pb-mqtt --test smoke -- --nocapture
echo "smoke ok"

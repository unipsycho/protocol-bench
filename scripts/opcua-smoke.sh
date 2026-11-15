#!/usr/bin/env bash
# Build pb-opcua into modules/, then optionally launch UI.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pb-opcua -p pb-ui
mkdir -p modules
cp -f target/debug/libpb_opcua.so modules/
cp -f target/debug/libpb_mqtt.so modules/ 2>/dev/null || true
echo "installed modules/libpb_opcua.so"
echo "Launch: PB_MODULES=\"$ROOT/modules\" DISPLAY=:0 cargo run -p pb-ui"

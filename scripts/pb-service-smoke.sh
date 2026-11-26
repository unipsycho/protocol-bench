#!/usr/bin/env bash
# Linux IPC smoke: start pb-service, hello+status over TCP, exit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pb-mqtt -p pb-opcua -p pb-streams -p pb-http -p pb-modbus -p pb-files -p pb-snmp -p pb-las -p pb-wits -p pb-websocket -p pb-sparkplug -p pb-dnp3 -p pb-can -p pb-service -q
mkdir -p modules
cp -f target/debug/libpb_mqtt.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_opcua.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_streams.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_http.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_modbus.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_files.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_snmp.so modules/ 2>/dev/null || true
cp -f target/debug/libpb_las.so modules/ 2>/dev/null || true

pkill -x pb-service 2>/dev/null || true
sleep 0.2
PB_MODULES="$ROOT/modules" PB_IPC_PORT=17500 \
  nohup "$ROOT/target/debug/pb-service" > /tmp/pb-service-smoke.log 2>&1 &
echo "service_pid=$!"
for i in $(seq 1 40); do
  if [[ -f "$HOME/.config/protocolbench/ipc.json" ]]; then
    break
  fi
  sleep 0.1
done
python3 - <<'PY'
import json, socket, struct, pathlib, sys
cfg = pathlib.Path.home()/".config/protocolbench"
ep = json.loads((cfg/"ipc.json").read_text())
token = (cfg/"ipc.token").read_text().strip()
host, port = ep["host"], int(ep["port"])
print(f"endpoint {host}:{port}")

def call(sock, method, params=None, id=1):
    body = json.dumps({"id": id, "method": method, "params": params or {}}).encode()
    sock.sendall(struct.pack("<I", len(body)) + body)
    hdr = sock.recv(4)
    assert len(hdr)==4
    n = struct.unpack("<I", hdr)[0]
    data = b""
    while len(data) < n:
        data += sock.recv(n - len(data))
    return json.loads(data)

s = socket.create_connection((host, port), timeout=5)
r = call(s, "hello", {"token": token, "client": "smoke"}, 1)
assert r.get("result", {}).get("ok"), r
r = call(s, "status", {}, 2)
print("status", json.dumps(r.get("result"), indent=2)[:500])
r = call(s, "ping", {}, 3)
assert r.get("result", {}).get("pong")
print("SMOKE_OK")
PY
pkill -x pb-service 2>/dev/null || true
echo "log:"; tail -n 8 /tmp/pb-service-smoke.log

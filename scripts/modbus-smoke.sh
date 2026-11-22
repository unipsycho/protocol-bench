#!/usr/bin/env bash
# End-to-end Modbus smoke: fake TCP device (python) <- pb-modbus pack <- pb-service -> IPC snapshot.
# Checks decoded f32 (CDAB) / u32 / bit / string values and a typed write back to the device.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pb-modbus -p pb-service -q
WORK="$(mktemp -d)"
trap 'pkill -x pb-service 2>/dev/null || true; rm -rf "$WORK"' EXIT
mkdir -p "$WORK/modules"
cp -f target/debug/libpb_modbus.so "$WORK/modules/"
# Private config dir: the service writes ipc.json there, so a running UI never attaches to this throwaway service.
export XDG_CONFIG_HOME="$WORK/cfg"
mkdir -p "$XDG_CONFIG_HOME"
export WORK ROOT
python3 - <<'PY'
import json, os, socket, struct, subprocess, threading, time, pathlib, signal, sys

work = pathlib.Path(os.environ["WORK"]); root = pathlib.Path(os.environ["ROOT"])
regs = {i: 0 for i in range(64)}
regs[0], regs[1] = 0xE979, 0x42F6          # 123.456f32, CDAB
regs[2], regs[3] = 0x0001, 0x86A0          # u32 100000
regs[4] = 0b1000                           # bit3
regs[5], regs[6] = 0x4869, 0x2100          # "Hi!"
coils = {i: False for i in range(16)}; coils[0] = True

def serve(conn):
    try:
        while True:
            h = conn.recv(7)
            if len(h) < 7: return
            tid, _, ln, unit = struct.unpack(">HHHB", h)
            pdu = conn.recv(ln - 1); fc = pdu[0]
            def ok(body): conn.sendall(struct.pack(">HHHB", tid, 0, len(body) + 1, unit) + body)
            def exc(code): ok(bytes([fc | 0x80, code]))
            a, n = struct.unpack(">HH", pdu[1:5]) if fc in (1, 3, 4, 15, 16) else (0, 0)
            if fc == 3:
                if any(x not in regs for x in range(a, a + n)): exc(2); continue
                ok(bytes([3, n * 2]) + b"".join(struct.pack(">H", regs[x]) for x in range(a, a + n)))
            elif fc == 4:
                exc(2)
            elif fc == 1:
                if any(x not in coils for x in range(a, a + n)): exc(2); continue
                bits = [coils[x] for x in range(a, a + n)]; by = bytearray((n + 7) // 8)
                for i, b in enumerate(bits):
                    if b: by[i // 8] |= 1 << (i % 8)
                ok(bytes([1, len(by)]) + bytes(by))
            elif fc == 5:
                addr, v = struct.unpack(">HH", pdu[1:5]); coils[addr] = v == 0xFF00; ok(pdu[:5])
            elif fc == 6:
                addr, v = struct.unpack(">HH", pdu[1:5]); regs[addr] = v; ok(pdu[:5])
            elif fc == 16:
                for i in range(n): regs[a + i] = struct.unpack(">H", pdu[6 + 2 * i:8 + 2 * i])[0]
                ok(pdu[:5])
            else: exc(1)
    finally:
        conn.close()

srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0)); srv.listen(); dev_port = srv.getsockname()[1]
_t = socket.socket(); _t.bind(("127.0.0.1", 0)); srv_port = _t.getsockname()[1]; _t.close()
threading.Thread(target=lambda: [threading.Thread(target=serve, args=(srv.accept()[0],), daemon=True).start() for _ in iter(int, 1)], daemon=True).start()

def bind(area, addr, dt, order="abcd", point=""):
    return {"area": area, "address": addr, "dtype": dt, "order": order, "unit": 0, "point": point}
port_in = {"id": 2, "protocol": "Modbus", "label": "PLC", "modbus_connection": "plc", "modbus_interval_secs": 0.2,
           "modbus_inputs": [bind("holding", 0, "f32", "cdab"), bind("holding", 2, "u32"), bind("holding", 4, "bit3"),
                             bind("holding", 5, "str2"), bind("coil", 0, "bool")], "modbus_outputs": []}
port_out = {"id": 3, "protocol": "Modbus", "label": "PLC out", "modbus_connection": "plc", "modbus_inputs": [],
            "modbus_outputs": [bind("holding", 20, "f32", "cdab", "setpoint"), bind("coil", 5, "bool", "abcd", "run")]}
project = {
    "version": 10, "points": [], "next_point_seq": 1,
    "modbus": {"selected_id": "", "connections": [
        {"id": "plc", "label": "PLC", "role": "client", "transport": "tcp",
         "host": "127.0.0.1", "port": dev_port, "unit_id": 1, "timeout_ms": 500, "retries": 2},
        {"id": "hub", "label": "Hub server", "role": "server", "transport": "tcp",
         "host": "127.0.0.1", "port": srv_port, "unit_id": 1, "map": [
            {"area": "holding", "address": 0, "dtype": "f32", "order": "abcd", "unit": 0, "point": "modbus_1"},
            {"area": "holding", "address": 2, "dtype": "u16", "order": "abcd", "unit": 0, "point": "from_master"}]}]},
    "hubs": {"hubs": [{"id": 1, "name": "Plant", "protocol": "Modbus", "modbus_server": "hub", "sources": [port_in], "outputs": [port_out],
                       "aggregators": [], "calculations": []}], "feeds": [], "next_id": 10},
}
(work / "project.json").write_text(json.dumps(project))
env = dict(os.environ, PB_MODULES=str(work / "modules"), PB_PROJECT=str(work / "project.json"), PB_IPC_PORT="17501")
svc = subprocess.Popen([str(root / "target/debug/pb-service")], env=env, stdout=open(work / "svc.log", "w"), stderr=subprocess.STDOUT)

cfg = pathlib.Path(os.environ["XDG_CONFIG_HOME"]) / "protocolbench"
for _ in range(60):
    if (cfg / "ipc.json").exists() and (cfg / "ipc.token").exists(): break
    time.sleep(0.1)
ep = json.loads((cfg / "ipc.json").read_text()); token = (cfg / "ipc.token").read_text().strip()

def call(sock, method, params=None, i=1):
    body = json.dumps({"id": i, "method": method, "params": params or {}}).encode()
    sock.sendall(struct.pack("<I", len(body)) + body)
    n = struct.unpack("<I", sock.recv(4))[0]; data = b""
    while len(data) < n: data += sock.recv(n - len(data))
    return json.loads(data)

fail = None
DBG = []
try:
    s = socket.create_connection((ep["host"], int(ep["port"])), timeout=5)
    assert call(s, "hello", {"token": token, "client": "modbus-smoke"})["result"]["ok"]
    pts = {}
    for _ in range(50):
        snap = call(s, "snapshot", {}, 2)["result"]
        pts = {p["source"]: p for p in snap["points"]}; DBG = snap["points"]
        if all(p["quality"] == "good" for p in pts.values()) and len(pts) >= 5: break
        time.sleep(0.2)
    def val(src): return pts[src]["value"]
    for src, want in [("modbus/plc::holding:0:f32:cdab", "123.456"), ("modbus/plc::holding:2:u32", "100000"),
                      ("modbus/plc::holding:4:bit3", "true"), ("modbus/plc::holding:5:str2", "Hi!"), ("modbus/plc::coil:0:bool", "true")]:
        got = val(src); print(f"{src} = {got}")
        assert want.lower() in got.lower(), (src, got, want)
    print("READ_OK")
    # Output writes: feed host points through the service and check the device.
    r = call(s, "submit", {"engine": "modbus", "command": {"op": "write_point", "name": "setpoint", "value": "98.6"}}, 3)
    r = call(s, "submit", {"engine": "modbus", "command": {"op": "write_point", "name": "run", "value": "true"}}, 4)
    for _ in range(40):
        if coils[5] and (regs[20] or regs[21]): break
        time.sleep(0.1)
    f = struct.unpack(">f", struct.pack(">HH", regs[21], regs[20]))[0]
    print(f"device setpoint={f:.2f} run={coils[5]}")
    assert abs(f - 98.6) < 0.01 and coils[5]
    print("WRITE_OK")
    # Server side: a master reads the value bridged from the device, and writes one back.
    def mb(sock, tid, pdu):
        sock.sendall(struct.pack(">HHHB", tid, 0, len(pdu) + 1, 1) + pdu)
        h = sock.recv(7); n = struct.unpack(">H", h[4:6])[0] - 1
        body = b""
        while len(body) < n: body += sock.recv(n - len(body))
        return body
    m = socket.create_connection(("127.0.0.1", srv_port), timeout=5)
    body = b""
    for _ in range(50):   # server mirrors modbus_1 once the engine pushes it
        body = mb(m, 1, struct.pack(">BHH", 3, 0, 2))
        if body[0] == 3 and struct.unpack(">f", body[2:6])[0] > 100: break
        time.sleep(0.2)
    bridged = struct.unpack(">f", body[2:6])[0]
    print(f"server serves modbus_1 = {bridged:.3f}")
    assert abs(bridged - 123.456) < 0.01
    assert mb(m, 2, struct.pack(">BHH", 3, 50, 1))[0] == 0x83      # unmapped -> exception
    assert mb(m, 3, struct.pack(">BHH", 6, 2, 321))[0] == 6         # master write accepted
    for _ in range(50):
        snap = call(s, "snapshot", {}, 9)["result"]
        got = [p for p in snap["points"] if p["name"] == "from_master"]
        if got and "321" in got[0]["value"]: break
        time.sleep(0.2)
    print("host point from_master =", got[0]["value"] if got else None)
    assert got and "321" in got[0]["value"]
    print("SERVER_OK")
    print("MODBUS_SMOKE_OK")
except Exception as e:
    fail = e; print("FAILED:", repr(e)); print(open(work / "svc.log").read()[-1500:])
finally:
    svc.send_signal(signal.SIGTERM)
sys.exit(1 if fail else 0)
PY

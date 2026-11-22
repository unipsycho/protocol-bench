#!/usr/bin/env bash
# End-to-end SNMP + LAS smoke through a real pb-service:
#  * a fake SNMP v2c agent (python, UDP): typed values, a changing counter, agent going silent / back
#  * a LAS 2.0 file followed as it grows (NULL value -> bad quality)
#  * a LAS file written from host points (depth-indexed rows)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pb-snmp -p pb-las -p pb-service -q
WORK="$(mktemp -d)"
trap 'pkill -x pb-service 2>/dev/null || true; rm -rf "$WORK"' EXIT
mkdir -p "$WORK/modules" "$WORK/cfg"
cp -f target/debug/libpb_snmp.so target/debug/libpb_las.so "$WORK/modules/"
# Private config dir: keeps the throwaway service's ipc.json away from a running UI.
export XDG_CONFIG_HOME="$WORK/cfg" WORK ROOT
python3 - <<'PY'
import json, os, pathlib, signal, socket, struct, subprocess, sys, threading, time

work = pathlib.Path(os.environ["WORK"]); root = pathlib.Path(os.environ["ROOT"])

# ---- minimal BER / SNMP agent -------------------------------------------------------------
def tlv(tag, body):
    n = len(body)
    ln = bytes([n]) if n < 128 else bytes([0x80 | (2 if n > 255 else 1)]) + n.to_bytes(2 if n > 255 else 1, "big")
    return bytes([tag]) + ln + body
def read(buf, i=0):
    tag = buf[i]; l = buf[i + 1]; i += 2
    if l & 0x80:
        n = l & 0x7F; l = int.from_bytes(buf[i:i + n], "big"); i += n
    return tag, buf[i:i + l], i + l
def enc_int(v):
    b = v.to_bytes((v.bit_length() + 8) // 8 or 1, "big", signed=True); return tlv(0x02, b)
def enc_oid(s):
    p = [int(x) for x in s.strip(".").split(".")]; body = bytes([p[0] * 40 + p[1]])
    for a in p[2:]:
        g = [a & 0x7F]; a >>= 7
        while a: g.append((a & 0x7F) | 0x80); a >>= 7
        body += bytes(reversed(g))
    return tlv(0x06, body)
def dec_oid(b):
    out = [b[0] // 40, b[0] % 40]; cur = 0
    for x in b[1:]:
        cur = (cur << 7) | (x & 0x7F)
        if not x & 0x80: out.append(cur); cur = 0
    return ".".join(map(str, out))
table = {
    "1.3.6.1.2.1.1.5.0": tlv(0x04, b"core-switch"),
    "1.3.6.1.2.1.1.3.0": tlv(0x43, (123456).to_bytes(3, "big")),
    "1.3.6.1.2.1.2.2.1.10.1": None,   # counter, set below
}
counter = [1000]; silent = [False]
def agent(sock):
    while True:
        data, addr = sock.recvfrom(65535)
        if silent[0]: continue
        _, msg, _ = read(data); _, ver, i = read(msg); _, comm, i = read(msg, i); _, pdu, _ = read(msg, i)
        _, rid, i = read(pdu); _, _, i = read(pdu, i); _, _, i = read(pdu, i); _, vbl, _ = read(pdu, i)
        vbs = b""; j = 0
        while j < len(vbl):
            _, vb, j = read(vbl, j); _, name, k = read(vb); oid = dec_oid(name)
            if oid == "1.3.6.1.2.1.2.2.1.10.1": val = tlv(0x41, counter[0].to_bytes(4, "big"))
            else: val = table.get(oid) or tlv(0x81, b"")
            vbs += tlv(0x30, enc_oid(oid) + val)
        resp = tlv(0xA2, tlv(0x02, rid) + enc_int(0) + enc_int(0) + tlv(0x30, vbs))
        sock.sendto(tlv(0x30, tlv(0x02, ver) + tlv(0x04, comm) + resp), addr)
usock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); usock.bind(("127.0.0.1", 0)); snmp_port = usock.getsockname()[1]
threading.Thread(target=agent, args=(usock,), daemon=True).start()

# ---- project -----------------------------------------------------------------------------
LAS_HEAD = """~Version Information
 VERS.   2.0 : CWLS
 WRAP.   NO  : one line per step
~Well Information
 NULL.   -999.25 : NULL
~Curve Information
 DEPT.M : depth
 GR  .GAPI : gamma
~ASCII Log Data
"""
las_in = work / "in.las"; las_out = work / "out.las"
las_in.write_text(LAS_HEAD + "100.0 50.5\n100.5 51.5\n")
project = {
    "version": 10, "points": [], "next_point_seq": 1,
    "snmp": {"selected_id": "", "targets": [{"id": "sw", "label": "Switch", "host": "127.0.0.1", "port": snmp_port,
             "version": "v2c", "community": "public", "timeout_secs": 1}]},
    "las": {"selected_id": "", "endpoints": [
        {"id": "well", "label": "Well", "role": "incoming", "path": str(las_in), "version": "2.0", "follow": True},
        {"id": "wout", "label": "Out", "role": "outgoing", "path": str(las_out), "version": "2.0", "follow": True}]},
    "hubs": {"hubs": [{"id": 1, "name": "H", "protocol": "Modbus", "aggregators": [], "calculations": [],
        "sources": [
          {"id": 2, "protocol": "Snmp", "label": "Switch", "snmp_target": "sw", "snmp_interval_secs": 0.3,
           "snmp_inputs": [{"oid": "1.3.6.1.2.1.1.5.0", "point": ""}, {"oid": "1.3.6.1.2.1.2.2.1.10.1", "point": ""},
                           {"oid": "1.3.6.1.2.1.1.3.0", "point": ""}, {"oid": "1.3.6.1.2.1.99.9", "point": ""}]},
          {"id": 3, "protocol": "Las", "label": "Well", "las_endpoint": "well", "las_index": "depth",
           "las_inputs": [{"curve": "GR", "point": ""}, {"curve": "DEPT", "point": ""}]}],
        "outputs": [
          {"id": 4, "protocol": "Las", "label": "Log out", "las_endpoint": "wout", "las_index": "depth",
           "las_outputs": [{"point": "depth", "curve": "DEPT"}, {"point": "gr_out", "curve": "GR"}]}]}],
        "feeds": [], "next_id": 10},
}
(work / "project.json").write_text(json.dumps(project))
cfg = pathlib.Path(os.environ["XDG_CONFIG_HOME"]) / "protocolbench"
env = dict(os.environ, PB_MODULES=str(work / "modules"), PB_PROJECT=str(work / "project.json"), PB_IPC_PORT="17504")
svc = subprocess.Popen([str(root / "target/debug/pb-service")], env=env, stdout=open(work / "svc.log", "w"), stderr=subprocess.STDOUT)
for _ in range(80):
    if (cfg / "ipc.json").exists() and (cfg / "ipc.token").exists(): break
    time.sleep(0.1)
ep = json.loads((cfg / "ipc.json").read_text()); token = (cfg / "ipc.token").read_text().strip()
def call(s, m, p=None, i=1):
    b = json.dumps({"id": i, "method": m, "params": p or {}}).encode(); s.sendall(struct.pack("<I", len(b)) + b)
    n = struct.unpack("<I", s.recv(4))[0]; d = b""
    while len(d) < n: d += s.recv(n - len(d))
    return json.loads(d)
def points(s): return {p["source"]: p for p in call(s, "snapshot", {}, 2)["result"]["points"]}
def wait_for(s, src, ok, what, secs=10):
    end = time.time() + secs
    while time.time() < end:
        p = points(s).get(src)
        if p and ok(p): return p
        time.sleep(0.1)
    raise AssertionError(f"{what}: {src} -> {points(s).get(src)}")
SYS, CNT, TIK, MISS = "snmp/sw::1.3.6.1.2.1.1.5.0", "snmp/sw::1.3.6.1.2.1.2.2.1.10.1", "snmp/sw::1.3.6.1.2.1.1.3.0", "snmp/sw::1.3.6.1.2.1.99.9"
GR, DEPT = "las/well::GR", "las/well::DEPT"
fail = None
try:
    s = socket.create_connection((ep["host"], int(ep["port"])), timeout=5)
    assert call(s, "hello", {"token": token, "client": "snmp-las"})["result"]["ok"]
    good = lambda p: p["quality"] == "good"
    wait_for(s, SYS, lambda p: good(p) and p["value"] == "core-switch", "sysName")
    wait_for(s, CNT, lambda p: good(p) and p["value"] == "1000", "counter")
    wait_for(s, TIK, lambda p: good(p) and p["value"] == "123456", "ticks")
    assert points(s)[MISS]["quality"] != "good", points(s)[MISS]
    print("SNMP typed values OK; missing OID not good:", points(s)[MISS]["quality"])
    counter[0] = 2468
    wait_for(s, CNT, lambda p: good(p) and p["value"] == "2468", "counter update")
    silent[0] = True
    wait_for(s, SYS, lambda p: p["quality"] == "bad" and p["value"] == "core-switch", "bad when agent silent", 20)
    print("silent agent -> bad quality, value held")
    silent[0] = False
    wait_for(s, SYS, good, "recovery", 20)
    print("agent back -> good again")
    print("SNMP_OK")
    wait_for(s, GR, lambda p: good(p) and p["value"].startswith("51.5"), "LAS follow initial")
    wait_for(s, DEPT, lambda p: p["value"].startswith("100.5"), "LAS depth")
    with las_in.open("a") as f: f.write("101.0 -999.25\n")
    wait_for(s, GR, lambda p: p["quality"] == "bad" and p["value"].startswith("51.5"), "NULL -> bad, value held")
    with las_in.open("a") as f: f.write("101.5 53.25\n")
    wait_for(s, GR, lambda p: good(p) and p["value"].startswith("53.25"), "LAS append")
    print("LAS follow OK: initial, append, NULL -> bad")
    for d, g in [(200.0, 10.5), (200.5, 11.5)]:
        call(s, "submit", {"engine": "las", "command": {"op": "write_point", "name": "gr_out", "value": str(g)}}, 3)
        call(s, "submit", {"engine": "las", "command": {"op": "write_point", "name": "depth", "value": str(d)}}, 4)
        time.sleep(0.6)
    end = time.time() + 8
    while time.time() < end and "200.5 11.5" not in (las_out.read_text() if las_out.exists() else ""): time.sleep(0.1)
    text = las_out.read_text(); print(text.split("~ASCII")[1].strip().replace("\n", " | "))
    assert "200 10.5" in text and "200.5 11.5" in text and text.count("~Curve") == 1
    print("LAS write OK")
    print("SNMP_LAS_SMOKE_OK")
except Exception as e:
    fail = e; print("FAILED:", repr(e)); print(open(work / "svc.log").read()[-1200:])
finally:
    svc.send_signal(signal.SIGTERM)
sys.exit(1 if fail else 0)
PY

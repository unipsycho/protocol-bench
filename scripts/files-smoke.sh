#!/usr/bin/env bash
# End-to-end File follow smoke: CSV appended to while pb-service runs, across a service restart
# (no re-read of seen rows, rows written while down are caught up) and a file rotation.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pb-files -p pb-service -q
WORK="$(mktemp -d)"
trap 'pkill -x pb-service 2>/dev/null || true; rm -rf "$WORK"' EXIT
mkdir -p "$WORK/modules" "$WORK/cfg"
cp -f target/debug/libpb_files.so "$WORK/modules/"
# Private config dir: keeps the throwaway service's ipc.json away from a running UI.
export XDG_CONFIG_HOME="$WORK/cfg" WORK ROOT
python3 - <<'PY'
import json, os, pathlib, signal, socket, struct, subprocess, sys, time

work = pathlib.Path(os.environ["WORK"]); root = pathlib.Path(os.environ["ROOT"])
csv = work / "meter.csv"
csv.write_text("time,temp,run\n10:00,20.5,true\n10:01,21.0,true\n")
project = {
    "version": 10, "points": [], "next_point_seq": 1,
    "files": {"selected_id": "", "sources": [{"id": "meter", "label": "Meter", "path": str(csv), "parser": "csv",
              "header_row": True, "delimiter": ",", "poll_ms": 100, "start_at": "beginning", "follow_rotation": True}]},
    "hubs": {"hubs": [{"id": 1, "name": "H", "protocol": "Modbus", "aggregators": [], "calculations": [], "outputs": [],
        "sources": [{"id": 2, "protocol": "Files", "label": "Meter log", "file_source": "meter",
                     "file_inputs": [{"column": "temp", "point": ""}, {"column": "run", "point": ""}]}]}],
        "feeds": [], "next_id": 10},
}
(work / "project.json").write_text(json.dumps(project))
cfg = pathlib.Path(os.environ["XDG_CONFIG_HOME"]) / "protocolbench"

def start(tag, project_file="project.json"):
    for f in ("ipc.json", "ipc.token"):
        (cfg / f).unlink(missing_ok=True)
    env = dict(os.environ, PB_MODULES=str(work / "modules"), PB_PROJECT=str(work / project_file),
               PB_IPC_PORT="17503")
    log = open(work / f"svc-{tag}.log", "w")
    p = subprocess.Popen([str(root / "target/debug/pb-service")], env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(80):
        if (cfg / "ipc.json").exists() and (cfg / "ipc.token").exists():
            break
        time.sleep(0.1)
    ep = json.loads((cfg / "ipc.json").read_text()); token = (cfg / "ipc.token").read_text().strip()
    s = socket.create_connection((ep["host"], int(ep["port"])), timeout=5)
    assert call(s, "hello", {"token": token, "client": "files-smoke"})["result"]["ok"]
    return p, s

def call(sock, method, params=None, i=1):
    body = json.dumps({"id": i, "method": method, "params": params or {}}).encode()
    sock.sendall(struct.pack("<I", len(body)) + body)
    n = struct.unpack("<I", sock.recv(4))[0]; data = b""
    while len(data) < n: data += sock.recv(n - len(data))
    return json.loads(data)

def points(s):
    return {p["source"]: p for p in call(s, "snapshot", {}, 2)["result"]["points"]}

def wait_for(s, src, want, what, secs=8):
    end = time.time() + secs
    while time.time() < end:
        p = points(s).get(src)
        if p and p["quality"] == "good" and want in p["value"]:
            return p
        time.sleep(0.1)
    raise AssertionError(f"{what}: {src} never became {want!r}; got {points(s).get(src)}")

def columns(s):
    for slot in call(s, "snapshot", {}, 3)["result"]["slots"]:
        if slot["id"] == "file_follow":
            for src in json.loads(slot["detail_json"] or "{}").get("sources", []):
                if src["id"] == "meter":
                    return {c["name"]: c["sample"] for c in src["columns"]}, src["state"]
    return {}, ""

TEMP, RUN = "file/meter::temp", "file/meter::run"
fail = None
svc = None
try:
    # Column picker: with NO binds the pack still reports the header + latest row's values.
    nobinds = json.loads((work / "project.json").read_text())
    nobinds["hubs"]["hubs"][0]["sources"][0]["file_inputs"] = []
    (work / "project-nobinds.json").write_text(json.dumps(nobinds))
    svc, s = start("p", "project-nobinds.json")
    end = time.time() + 10
    cols, state = {}, ""
    while time.time() < end:
        cols, state = columns(s)
        if cols.get("temp") == "21.0": break
        time.sleep(0.2)
    assert cols == {"time": "10:01", "temp": "21.0", "run": "true"}, (cols, state)
    assert "preview" in state, state
    assert not points(s).get("file/meter::temp"), "preview must not publish points"
    print("column picker (no binds):", cols, "|", state)
    svc.send_signal(signal.SIGTERM); svc.wait(timeout=10)
    (cfg / "file-follow").exists() and [f.unlink() for f in (cfg / "file-follow").glob("*.json")]

    svc, s = start("a")
    wait_for(s, TEMP, "21", "initial rows (start_at=beginning)")
    end = time.time() + 10   # slot status is published on the engine's next tick, a bit after the point
    while time.time() < end and "temp" not in columns(s)[0]:
        time.sleep(0.1)
    assert "temp" in columns(s)[0], columns(s)
    print("column picker (following):", columns(s)[0])
    print("initial rows read:", points(s)[TEMP]["value"])
    with csv.open("a") as f: f.write("10:02,22.5,false\n")
    wait_for(s, TEMP, "22.5", "append while running"); wait_for(s, RUN, "false", "bool column")
    print("appended row ->", points(s)[TEMP]["value"], points(s)[RUN]["value"])
    # Half-written line is not published until its newline arrives.
    with csv.open("a") as f: f.write("10:03,23.")
    time.sleep(0.5); assert "23" not in points(s)[TEMP]["value"][:2], points(s)[TEMP]
    with csv.open("a") as f: f.write("5,true\n")
    wait_for(s, TEMP, "23.5", "completed partial line")
    print("partial line held until complete: OK")
    svc.send_signal(signal.SIGTERM); svc.wait(timeout=10)

    # Rows written while the service is down are caught up; nothing is read twice.
    with csv.open("a") as f: f.write("10:04,24.5,true\n10:05,25.5,true\n")
    svc, s = start("b")
    wait_for(s, TEMP, "25.5", "catch-up after restart")
    log_b = (work / "svc-b.log").read_text()
    print("after restart:", points(s)[TEMP]["value"])
    assert "+2 row(s)" in log_b, log_b[-800:]
    assert "+6 row(s)" not in log_b and "+5 row(s)" not in log_b and "+4 row(s)" not in log_b, "old rows were re-read"
    print("no re-read after restart: OK (read exactly 2 new rows)")

    # Rotation: file replaced with a new one whose columns are in a different order.
    csv.rename(work / "meter.csv.1")
    csv.write_text("run,temp,time\nfalse,30.5,11:00\n")
    wait_for(s, TEMP, "30.5", "rotation + new header")
    print("rotated file with new header order ->", points(s)[TEMP]["value"])
    print("FILES_SMOKE_OK")
except Exception as e:
    fail = e
    print("FAILED:", repr(e))
    for f in sorted(work.glob("svc-*.log")): print(f.name, "\n", f.read_text()[-800:])
finally:
    if svc: svc.send_signal(signal.SIGTERM)
sys.exit(1 if fail else 0)
PY

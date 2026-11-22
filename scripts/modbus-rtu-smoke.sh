#!/usr/bin/env bash
# Modbus RTU over real serial devices, no hardware: a pair of linked pseudo-terminals (a virtual
# null-modem cable, made with python's pty module — no socat needed) carries an RTU slave on one
# end and an RTU master on the other, through the same serial driver + framing the pack uses.
# Re-runnable: creates and removes the pair every time. Linux / macOS.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo test -p pb-modbus --test rtu_pty --no-run -q
python3 - <<'PY'
import os, pty, select, subprocess, sys, termios, threading, tty

# Two ptys; whatever one side writes the other side reads.
m1, s1 = pty.openpty(); m2, s2 = pty.openpty()
for fd in (s1, s2):
    tty.setraw(fd)  # raw: no echo / line discipline between the "cable" and the driver
names = (os.ttyname(s1), os.ttyname(s2))
print("virtual cable:", names[0], "<->", names[1])
stop = threading.Event()
def pump(src, dst):
    while not stop.is_set():
        r, _, _ = select.select([src], [], [], 0.1)
        if r:
            try:
                data = os.read(src, 4096)
            except OSError:
                return
            if data:
                os.write(dst, data)
threading.Thread(target=pump, args=(m1, m2), daemon=True).start()
threading.Thread(target=pump, args=(m2, m1), daemon=True).start()
env = dict(os.environ, PB_RTU_PORTS=f"{names[0]},{names[1]}")
rc = subprocess.run(["cargo", "test", "-p", "pb-modbus", "--test", "rtu_pty", "-q", "--", "--ignored", "--nocapture"], env=env).returncode
stop.set()
print("RTU_PTY_SMOKE_OK" if rc == 0 else "RTU_PTY_SMOKE_FAILED")
sys.exit(rc)
PY

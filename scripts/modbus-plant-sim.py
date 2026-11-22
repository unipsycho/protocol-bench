#!/usr/bin/env python3
"""Fake tank PLC for the Protocol Bench Plant sample (Modbus TCP, unit 1, port 5020).

Register map (matches the sample hub's "Tank PLC" Input / Output binds):
  holding 100-101  f32 CDAB   tank level %      (slow sine, 20..80)
  holding 102-103  f32 CDAB   tank temp  degC   (18..24)
  holding 110-111  u32 ABCD   flow total        (counts up)
  holding 120      bit 3      alarm (level > 78 or < 22)
  coil 0           pump running (follows coil 1 = pump command)
  coil 1           pump command  <- written by the hub's "Pump command" Output
  discrete 0       high-level switch (level > 75)

Usage: scripts/modbus-plant-sim.py [port]
"""
import math, socket, struct, sys, threading, time

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 5020
UNIT = 1
regs = {a: 0 for a in range(0, 200)}
coils = {0: True, 1: True, 2: False, 3: False}
discrete = {0: False, 1: False}
lock = threading.Lock()


def f32_cdab(x):
    hi, lo = struct.unpack(">HH", struct.pack(">f", x))
    return lo, hi  # CDAB: low word first


def simulate():
    t0 = time.time()
    flow = 0.0
    while True:
        t = time.time() - t0
        level = 50 + 30 * math.sin(t / 20.0)
        temp = 21 + 3 * math.sin(t / 45.0)
        with lock:
            coils[0] = coils[1]  # pump follows its command
            if coils[0]:
                flow += 0.5 * 12.0
            regs[100], regs[101] = f32_cdab(level)
            regs[102], regs[103] = f32_cdab(temp)
            regs[110], regs[111] = (int(flow) >> 16) & 0xFFFF, int(flow) & 0xFFFF
            alarm = level > 78 or level < 22
            regs[120] = (regs[120] & ~(1 << 3)) | (int(alarm) << 3)
            discrete[0] = level > 75
        time.sleep(0.5)


def handle(pdu):
    fc = pdu[0]
    exc = lambda code: bytes([fc | 0x80, code])
    with lock:
        if fc in (1, 2, 3, 4):
            addr, n = struct.unpack(">HH", pdu[1:5])
            if fc in (1, 2):
                src = coils if fc == 1 else discrete
                if n < 1 or n > 2000 or any(a not in src for a in range(addr, addr + n)):
                    return exc(2)
                out = bytearray((n + 7) // 8)
                for i in range(n):
                    if src[addr + i]:
                        out[i // 8] |= 1 << (i % 8)
                return bytes([fc, len(out)]) + bytes(out)
            if n < 1 or n > 125 or any(a not in regs for a in range(addr, addr + n)):
                return exc(2)
            return bytes([fc, n * 2]) + b"".join(struct.pack(">H", regs[a]) for a in range(addr, addr + n))
        if fc == 5:
            addr, v = struct.unpack(">HH", pdu[1:5])
            if addr not in coils:
                return exc(2)
            coils[addr] = v == 0xFF00
            return pdu[:5]
        if fc == 6:
            addr, v = struct.unpack(">HH", pdu[1:5])
            if addr not in regs:
                return exc(2)
            regs[addr] = v
            return pdu[:5]
        if fc == 15:
            addr, n = struct.unpack(">HH", pdu[1:5])
            if any(a not in coils for a in range(addr, addr + n)):
                return exc(2)
            for i in range(n):
                coils[addr + i] = bool(pdu[6 + i // 8] >> (i % 8) & 1)
            return pdu[:5]
        if fc == 16:
            addr, n = struct.unpack(">HH", pdu[1:5])
            if any(a not in regs for a in range(addr, addr + n)):
                return exc(2)
            for i in range(n):
                regs[addr + i] = struct.unpack(">H", pdu[6 + 2 * i:8 + 2 * i])[0]
            return pdu[:5]
    return exc(1)


def recv_exact(conn, n):
    buf = b""
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            raise ConnectionError
        buf += chunk
    return buf


def serve(conn):
    try:
        while True:
            tid, _pid, ln, unit = struct.unpack(">HHHB", recv_exact(conn, 7))
            pdu = recv_exact(conn, ln - 1)
            if unit != UNIT:
                continue
            body = handle(pdu)
            conn.sendall(struct.pack(">HHHB", tid, 0, len(body) + 1, unit) + body)
    except (ConnectionError, OSError):
        pass
    finally:
        conn.close()


if __name__ == "__main__":
    threading.Thread(target=simulate, daemon=True).start()
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", PORT))
    srv.listen()
    print(f"tank PLC simulator on 127.0.0.1:{PORT} (unit {UNIT}) — Ctrl-C to stop")
    while True:
        c, _ = srv.accept()
        threading.Thread(target=serve, args=(c,), daemon=True).start()

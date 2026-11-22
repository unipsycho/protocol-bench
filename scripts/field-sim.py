#!/usr/bin/env python3
"""Field simulator for the Modbus-hub and Edge-logger demo projects.

  * SNMP v2c agent on 127.0.0.1:1161 (community any): sysUpTime, sysName, ifInOctets.1 (counter)
  * data/meter.csv   - time,volts,kw,temp  (one row per second, appended)
  * data/well-a.las  - LAS 2.0 well log, one depth row per second (DEPT,GR,DT)

Usage: scripts/field-sim.py [data-dir]     (default ../protocol-bench-projects/data)
Stop with Ctrl-C. Files are recreated on every start.
"""
import math, os, pathlib, random, socket, sys, threading, time

root = pathlib.Path(__file__).resolve().parent.parent
data = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else root.parent / "protocol-bench-projects" / "data"
data.mkdir(parents=True, exist_ok=True)
PORT = int(os.environ.get("FIELD_SIM_SNMP_PORT", "1161"))

# ---- SNMP agent (BER by hand, v1/v2c GET) -----------------------------------------------
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
    return tlv(0x02, v.to_bytes((v.bit_length() + 8) // 8 or 1, "big", signed=True))
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
def uint(tag, v):
    return tlv(tag, v.to_bytes(max(1, (v.bit_length() + 7) // 8), "big"))

t0 = time.time()
def value(oid):
    up = int((time.time() - t0) * 100)
    if oid == "1.3.6.1.2.1.1.5.0": return tlv(0x04, b"core-switch")
    if oid == "1.3.6.1.2.1.1.3.0": return uint(0x43, up)                        # TimeTicks
    if oid == "1.3.6.1.2.1.2.2.1.10.1": return uint(0x41, 1000 + up * 37 % 2**32)  # Counter32
    return tlv(0x80, b"")                                                          # noSuchObject

def agent(sock):
    while True:
        data_, addr = sock.recvfrom(65535)
        try:
            _, msg, _ = read(data_); _, ver, i = read(msg); _, comm, i = read(msg, i); _, pdu, _ = read(msg, i)
            _, rid, i = read(pdu); _, _, i = read(pdu, i); _, _, i = read(pdu, i); _, vbl, _ = read(pdu, i)
            vbs = b""; j = 0
            while j < len(vbl):
                _, vb, j = read(vbl, j); _, name, _ = read(vb); oid = dec_oid(name)
                vbs += tlv(0x30, enc_oid(oid) + value(oid))
            resp = tlv(0xA2, tlv(0x02, rid) + enc_int(0) + enc_int(0) + tlv(0x30, vbs))
            sock.sendto(tlv(0x30, tlv(0x02, ver) + tlv(0x04, comm) + resp), addr)
        except Exception as e:  # malformed request: ignore
            print("snmp: bad request", e)

usock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
usock.bind(("127.0.0.1", PORT))
threading.Thread(target=agent, args=(usock,), daemon=True).start()
print(f"snmp agent on udp 127.0.0.1:{PORT}")

# ---- growing files ----------------------------------------------------------------------
meter = data / "meter.csv"
meter.write_text("time,volts,kw,temp\n")
las = data / "well-a.las"
las.write_text("""~Version Information
 VERS.   2.0 : CWLS LOG ASCII STANDARD - VERSION 2.0
 WRAP.   NO  : ONE LINE PER DEPTH STEP
~Well Information
 STRT.M   1000.0 : START DEPTH
 STOP.M   1000.0 : STOP DEPTH
 STEP.M   0.5    : STEP
 NULL.    -999.25 : NULL VALUE
 WELL.    FIELD-SIM WELL A : WELL
~Curve Information
 DEPT.M                 : 1  DEPTH
 GR  .GAPI              : 2  GAMMA RAY
 DT  .US/M              : 3  SONIC TRANSIT TIME
~ASCII Log Data
""")
print(f"meter csv   {meter}\nwell-a las  {las}\nCtrl-C to stop")

depth, n = 1000.0, 0
try:
    while True:
        t = time.time() - t0
        volts = 400 + 8 * math.sin(t / 7) + random.uniform(-0.5, 0.5)
        kw = 12 + 3 * math.sin(t / 11) + random.uniform(-0.2, 0.2)
        temp = 41 + 2 * math.sin(t / 30)
        with meter.open("a") as f:
            f.write(f"{time.strftime('%H:%M:%S')},{volts:.2f},{kw:.2f},{temp:.2f}\n")
        gr = 80 + 40 * math.sin(depth / 9) + random.uniform(-2, 2)
        dt_ = 95 + 15 * math.cos(depth / 13)
        row = f"{depth:.1f} {gr:.2f} {dt_:.2f}" if n % 15 != 14 else f"{depth:.1f} {gr:.2f} -999.25"  # an occasional NULL
        with las.open("a") as f:
            f.write(row + "\n")
        depth += 0.5; n += 1
        time.sleep(1)
except KeyboardInterrupt:
    pass

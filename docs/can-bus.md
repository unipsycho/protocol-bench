# CAN bus

Status as of 2026-11-27. The `pb-can` pack reads signals from CAN frames into points (hub **Input**) and packs points into CAN frames (hub **Output**). It's all Rust: no C libraries, vendor DLLs or outside tools.

Code: `pb-proto/src/can.rs` (signals, frame ids, `CanBus` interface, SocketCAN), `pb-proto/src/can/virt.rs` (virtual bus), `pb-proto/src/can/pcan.rs` (PCAN gateway), `pb-can/src/pack.rs` (the worker), `pb-ui/src/ext_proto.rs` (the `CAN` form).

## Transports

Each CAN connection picks one **transport**. Signals, frame ids and binds work the same on all of them, so a project can switch from the virtual bus to real hardware by changing only its connections.

| Transport | `transport` value | Talks to | Platforms | Setup |
| --- | --- | --- | --- | --- |
| **SocketCAN** | `socketcan` (default) | Linux CAN interfaces: `can0` (USB PCAN and other adapters with a kernel driver) or `vcan0` | Linux only | Interface must exist and be up; bit rate is set on the interface, not in Protocol Bench |
| **Virtual bus** | `virtual` | Other CAN connections in the same `pb-service` with the same bus name | All | None: no root, no kernel module |
| **PCAN Ethernet gateway** | `pcan_gw` | PEAK PCAN-Ethernet Gateway DR, PCAN-Gateway FD and similar, over UDP or TCP | All | Routes set up in the gateway's web page (see below) |

Projects saved before transports existed have no `transport` field and stay on SocketCAN, so the frac spread demo's J1939 engine is unchanged.

**Rule on every transport:** a connection never receives the frames it sent. To read back your own output (loopback testing), use two connections: one sends, the other receives. SocketCAN works the same way.

### Connection fields

| Field | Used by | Default | Meaning |
| --- | --- | --- | --- |
| `transport` | all | `socketcan` | `socketcan`, `virtual` or `pcan_gw` |
| `interface` | SocketCAN | `vcan0` (new connections) | `can0`, `can1`, `vcan0`, … |
| `bus` | Virtual | `loop` | Bus name; connections with the same name share frames |
| `host` | PCAN gateway | — (required) | Gateway IP address or host name |
| `protocol` | PCAN gateway | `udp` | `udp` or `tcp`, matching the gateway's routes |
| `port` | PCAN gateway | `55002` | UDP: the port of the gateway's LAN → CAN route. TCP: the route's port |
| `local_port` | PCAN gateway, UDP | `55001` | Port this PC listens on; the gateway's CAN → LAN route sends here |
| `stale_secs` | all | `5` | No matching frame for this long turns an Input point bad |

The connection list shows a summary line, for example `SocketCAN can0`, `Virtual bus "loop"`, `PCAN gateway udp://192.168.1.10:55002 · listen 55001` or `PCAN gateway tcp://192.168.1.10:55002`.

### SocketCAN (Linux)

Uses a raw CAN socket on the named interface. Error frames and remote (RTR) frames are ignored. If the interface is missing or goes away, the connection shows the error and retries with back-off (up to 10 s).

A virtual kernel interface for testing needs root once per boot:

```bash
sudo modprobe vcan && sudo ip link add dev vcan0 type vcan && sudo ip link set up vcan0
```

To have it at every boot, add a systemd-networkd `.netdev` file (`Kind=vcan`, `Name=vcan0`) and a matching `.network` file, and load the module from `/etc/modules-load.d/vcan.conf`. A real adapter needs its bit rate set, for example `sudo ip link set can0 type can bitrate 250000 && sudo ip link set up can0`.

### Virtual bus

An in-memory bus inside the CAN pack. Every connection that names the same bus gets every frame the others send. It needs nothing installed and works on Windows, macOS and Linux.

Limit: it only reaches connections inside the same `pb-service`. `candump`, another program or another machine can't see it. Use SocketCAN `vcan0` or a PCAN gateway for that.

### PCAN Ethernet gateway

The gateway forwards CAN traffic to the PC over the network. It needs no driver on the PC. You configure it as **routes** in its web page:

- **UDP** (two routes):
  - **CAN → LAN**: aimed at this PC's IP, on the connection's `local_port` (default 55001).
  - **LAN → CAN**: the gateway listens on the connection's `port` (default 55002).
- **TCP** (one route): one connection to `host:port` carries both directions. Protocol Bench connects as the client, so set the gateway's end of the route to accept the connection (server). If the connection drops, it reconnects with back-off.

Bit rate, CAN channel and filters are set on the gateway. Each route decides which CAN channel it serves. Protocol Bench sends channel 0 and ignores the channel byte on frames it receives.

Wire format: one big-endian record per frame (36 bytes for a standard CAN frame), several per UDP datagram or back to back on TCP:

| Bytes | Field |
| --- | --- |
| 0–1 | Record length, including these two bytes |
| 2–3 | Type: `0x80` CAN, `0x81` CAN + CRC32, `0x90` CAN FD, `0x91` CAN FD + CRC32 |
| 4–11 | Tag (0) |
| 12–19 | Timestamp, µs (we send 0) |
| 20 | Channel |
| 21 | DLC (CAN FD: DLC code 0–15) |
| 22–23 | Flags |
| 24–27 | CAN id; bit 31 = 29-bit extended, bit 30 = RTR |
| 28– | Data, then the CRC32 if the type has one |

| Direction | Supported |
| --- | --- |
| Send | Standard CAN frames (`0x80`), up to 8 bytes, 11- or 29-bit ids |
| Receive | `0x80`, `0x81`, plus `0x90` / `0x91` CAN FD frames with up to 8 data bytes |
| Receive, skipped | CAN FD frames over 8 bytes, remote (RTR) frames, unknown record types |
| Not checked | CRC32 values |

On UDP, a bad datagram is dropped. On TCP, a stream that loses its place reconnects.

**Caveat:** UDP has no connection, so a UDP gateway connection shows as up even if the gateway is off or the routes are wrong. In that case Input points go bad after `stale_secs`. Check the gateway's routes and the PC's firewall (UDP `local_port` inbound).

The format follows PEAK's published gateway documentation and is covered by unit tests. It hasn't yet been tested against a real gateway.

## Signals

A signal is a bit field in the 8-byte payload, DBC style. Input and Output binds use the same fields:

| Field | Values | Meaning |
| --- | --- | --- |
| `id` | `0x123`, `0x18FEF100`, `291`, `pgn:61444` | Frame id (see below) |
| `start` | 0–63 | Start bit. Intel: least significant bit. Motorola: most significant bit, DBC numbering |
| `len` | 1–64 | Length in bits |
| `order` | `intel` (default), `motorola` | Byte order: little-endian / big-endian |
| `type` | `unsigned` (default), `signed`, `float` | `float` = IEEE float32, must be 32 bits |
| `scale`, `offset` | numbers (1, 0) | value = raw × scale + offset |
| `dlc` | 0–8 (8) | Output only: data length of the frame sent |

An Intel signal must fit inside the 8 bytes. Bad binds (bad id, length out of range, float not 32 bits) are reported in the connection status and skipped.

**Frame ids:**
- **11-bit:** up to three hex digits, e.g. `0x100`, or a decimal number up to 2047.
- **29-bit (extended):** any id above `0x7FF`, any hex id written with four or more digits (e.g. `0x00000100` is the 29-bit id 0x100), or a trailing `x` (e.g. `256x`).
- **J1939:** `pgn:61444` or `pgn:0xF004` matches a parameter group from any source address and priority, for 29-bit frames only. PDU1 groups (PF < 240) ignore the destination byte. **Input only.** Output binds need an exact id; a `pgn:` Output bind is ignored.

### Input (frames → points)

- Every frame that matches a bind's id updates that point. Several binds can read different bits of the same frame.
- The point is an integer when scale is 1, offset is 0 and the type isn't float; otherwise it's a decimal number. A 1-bit signal reads as `0` / `1`, not true / false.
- No matching frame for `stale_secs` turns the point bad until frames come back.
- The CAN panel's "seen on the wire" table lists every frame id received, with its J1939 PGN for 29-bit ids and the last data (`18FEF100#0011…`). You can click it to add binds.

### Output (points → frames)

- Each frame id has one 8-byte buffer. Each Output bind writes its point's value into its bits. Bits no bind covers stay **0**. Note that J1939 normally fills unused bytes with `0xFF`.
- A frame starts being sent once any of its signals has a value, then every **Transmit every** interval of the Output node (default 0.1 s) with the latest values. All frames on one Output node share that interval.
- Values out of range are clamped: e.g. 300 into an 8-bit unsigned field sends 255, and −300 into an 8-bit signed field sends −128. True / false points send 1 / 0.
- The frame's DLC comes from the first bind for that id (default 8).

## Loopback demo

`protocol-bench-projects/can-loopback.json` tests the whole path with no hardware:

1. **Generators:** six Generator nodes make 20 signals.
2. **Send:** a CAN Output on connection `bus_tx` sends them as six frames every 100 ms on the virtual bus `loop`.
3. **Read back:** a CAN Input on connection `bus_rx` on the same bus reads them back as `rx_<signal>`.
4. **MQTT:** the hub publishes the `rx_` points to its embedded broker (`:1888`) as `can/rx/<signal>`.

The frames cover:
- 11-bit ids in Intel order (`0x100`) and Motorola order (`0x101`)
- 29-bit ids with float32 (`0x18FF1000`) and mixed widths with a signed field (`0x18FF1100`)
- J1939 EEC1 and ET1 (`0x0CF00400`, `0x18FEEE00`), read back by `pgn:61444` / `pgn:65262` with J1939 scaling
- single-bit flags and a 2-byte frame (`0x200`)

To run the same signals over a kernel or real bus, change both connections' transport, for example to SocketCAN `vcan0`. On a real bus, send on one adapter or gateway and receive on another.

Results: all 40 points good, every `rx_` value matches its generator value within the field's resolution, and all `can/rx/…` topics published.

## Tests

| Where | What |
| --- | --- |
| `pb-proto` `can::tests` | Intel / Motorola encode + decode, signed, float32, clamping, id and PGN parsing |
| `pb-proto` `can::virt::tests` | Frames reach other ports on the bus, not the sender or other buses; ports leave on drop |
| `pb-proto` `can::pcan::tests` | Record layout, several records per buffer, CRC and FD records, skipped records, partial TCP records; UDP and TCP round trips against a fake gateway |
| `pb-can` `pack::tests` | Two virtual connections through the real worker (Motorola signed + J1939 PGN); the worker against a fake UDP gateway; transport checks and summaries; SocketCAN `vcan0` round trip (runs only when `vcan0` exists) |
| `pb-ui` `demos::tests::can_loopback_is_consistent` | The demo: 20 signals, every one sent and read back, separate send / receive connections |

## Not supported (yet)

| Area | Gap |
| --- | --- |
| CAN FD | No sending; frames over 8 data bytes are skipped when received (PCAN gateway). SocketCAN reads standard frames only |
| Frame kinds | Remote (RTR) and error frames are ignored; no way to send them |
| J1939 | No multi-packet transport (TP.CM / BAM), address claim or request PGN; `pgn:` matching is Input only |
| Signal definitions | No DBC file import, multiplexed signals or value tables; signals are entered by hand |
| Sending | One interval per Output node; no per-frame rates and no send-on-change. Unused bits are 0, not `0xFF` |
| Read-back values | Scaled values can show float noise (e.g. `6.300000000000001` for 6.3 at scale 0.1) and float32 fields show float32 precision |
| Hardware | No SLCAN (USB serial adapters such as CANable), no PCAN-Basic (USB PCAN on Windows), no Kvaser or Vector |
| Bus setup | Bit rate and filters aren't set from Protocol Bench (set them on the interface or gateway) |
| Virtual bus reach | In-process only; no CAN-over-IP bridge to other programs or a simulator |

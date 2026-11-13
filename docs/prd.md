# Protocol Bench — product requirements

Date: 2026-11-13. Status: living draft.

Research: [competitive-landscape.md](competitive-landscape.md) (store). Free vs paid isolation: [free-vs-paid-isolation.md](free-vs-paid-isolation.md). Protocol notes: [protocol-options.md](protocol-options.md).

Checkout: `/home/mike/Documents/AAI-Projects/protocol bench` (workspace `pb-ui` + `pb-engine`, egui shell).

## Product

Protocol Bench is a native Rust bench for moving values between industrial and oilfield links. One process holds the points. A value that arrives on one protocol can leave on another. The screen and the converter live in the same project.

This build is the shell. Live protocol I/O comes later, on the engine threads. The window is egui (`eframe`), not a web UI. Interaction follows Modbus Bench, with a Protocol Bench look.

Protocol Bench is a **new product**, separate from Modbus Bench. Modbus Bench remains the Modbus-only workbench; Protocol Bench can be tested against it.

## Problem

Moving a value between a serial instrument, a Modbus device, a growing CSV, an MQTT broker, and an OPC UA client today means a chain of single-purpose tools, a hardware gateway sold per protocol pair, or an industrial connectivity server licensed by driver and tag count. The small job — several COM ports in, a file that a meter keeps appending, a TCP Modbus map and an MQTT topic set out — is either hand-wired or priced like a plant server.

## Users

- An integration engineer connecting meters, serial instruments, and Modbus devices to MQTT or OPC UA.
- A technician who currently keeps a serial terminal, a Modbus master, and an MQTT client open at the same time.
- Someone whose instrument or logger only appends rows to a CSV or JSON-lines file.
- A site that needs one always-on converter for a handful of ports, with a screen that shows the live points.
- Oilfield / frac data plane users who need LAS files and WITS record streams beside the industrial set.

## Shell (today)

- Docked panels (drag titles, tab strips, edge splits). Top bar: File, View, Help, theme chip, settings. Side rail of panel icons.
- Themes. Hub canvas and panel diagrams keep the scope blue mixed with the theme background.
- Help → About: Agile Automation logo, “Protocol Bench” wordmark, version.
- Panels: Project, Hubs, Points, Mappings, Traffic, Streams, Modbus, MQTT, OPC UA, File follow.

## Engines

Each protocol engine owns its work on a thread the UI does not block. `pb-engine` is that boundary: the UI takes a snapshot and submits commands. The five slots that exist now (Streams, Modbus, MQTT, OPC UA, File follow) are idle — they do not open a serial port, a socket, or a file.

## Hubs

A hub is a conversion point. Sources come in. Outputs leave. A diagram shows what converts to what.

- More than one hub; a hub can be split across protocols or locked to one.
- Arrange the diagram: drag nodes, click for details, open the matching protocol panel from a protocol node.
- Sample hubs ship for framing (Plant split + Edge single-protocol, MQTT link between them) — configuration only, not a live bridge.

## What runs today, and what does not

Today: the window, layout, themes, the hub diagram, and the engine thread boundary.

Not yet: opening a port, speaking a protocol, parsing a file, or writing a point. No file requester in this build. When one is added, it must not block the UI thread (blocking dialogs / vsync stalls on another Hyprland workspace trip “not responding”).

## Protocols

Already named in the shell:

- Raw serial (Streams) — first build: **both** transparent byte pipe and delimited frames
- Modbus — first build: **client and server**
- MQTT — first build: **client** required; **embedded broker / in-process server** planned for hub scenarios (MQTT first; consider OPC UA for similar hub concepts)
- OPC UA — first build: **client and server**
- File follow — first build parsers: **CSV and JSON lines**

Also in the product, not all drawn as their own panels yet:

1. Serial ASCII and custom line protocols
2. HTTP REST / JSON gateways
3. WebSocket
4. Sparkplug B
5. DNP3
6. SNMP
7. CAN / SocketCAN
8. LAS
9. WITS

LAS and WITS are the oilfield and frac data plane. WITS means the WITS record stream, not WITSML. WITSML is a later, separate decision.

## First usable build (targets)

When live engines land, the first usable build should cover:

| Endpoint | Role |
| --- | --- |
| Serial port | Transparent byte pipe **and** delimited frames; also transport for Modbus RTU |
| TCP socket | Client and listener, raw streams and Modbus TCP |
| Modbus RTU / TCP | **Client and server** (master/slave and TCP client/server) |
| MQTT | **Client** publish and subscribe; embedded broker capability for hub scenarios |
| OPC UA | **Client and server** |
| File follow | Growing CSV / JSON lines → points; byte offset across restart and rotate |
| Platforms | **Linux and Windows** |
| Mappings | **Type-preserving copy only** (no scale / endian / word-swap yet) |

Done when:

- Two serial ports and two TCP endpoints run in one project.
- A Modbus value is visible as a point and leaves on MQTT and OPC UA; MQTT can write a Modbus register.
- Stopping and opening the project restores the same bridges.
- A dead endpoint shows bad quality and is retried.
- Appending a CSV row becomes points that leave on MQTT without rereading seen rows.
- The same project shape runs on Linux and Windows.

## Multi-hub test approach

Bench validation uses **scripts → hub store → outputs**, then a **second hub** that imports those outputs. With bi-directional protocols, exercise **many-to-many** flows and **multi-hub** setups. Modbus Bench can participate as an external far end when useful. Prefer this path over a separate “simulate far end” feature for first-build validation.

## Later

Hold until the first build is real: SQL, cloud connectors, Kafka as its own client, Influx line protocol as a write format, syslog / NATS / MTConnect / OpenTelemetry, vendor PLC catalogs (S7, EtherNet/IP, BACnet, …), OPC DA / COM, historians, redundancy, OPC Foundation certification, scripted transforms beyond type-preserving copy (scale / endian / word-swap), WITSML.

Embedded MQTT broker (and similar in-process server ideas, e.g. OPC UA) are **planned for hub scenarios**, not deferred forever — see Settled decisions.

## Market position

Short version (detail in the landscape doc): Neuron, Kepware, and DataHub are plant connectivity servers; HighByte and HiveMQ Edge assume OPC/MQTT-shaped data; Node-RED / Telegraf / PLC4X are glue. Protocol Bench takes serial + Modbus + MQTT + OPC UA + file follow (and oilfield LAS/WITS) as one native Rust process with the bench and converter in the same project.

## Runtime

- Language: Rust end to end. Workspace: `pb-ui` (egui/eframe), `pb-engine`.
- Candidate crates (spikes, not defaults): `serialport`, `tokio-modbus`, `rumqttc` (or equivalent), [async-opcua](https://github.com/FreeOpcUa/async-opcua) (MPL-2.0 subset).
- Adjacent product: Modbus Bench remains Modbus-only; Protocol Bench is separate and can be tested with it.

## Settled decisions

Recorded 2026-11-13 from Mike’s answers (numbering matches the former Open decisions list):

2. **UI toolkit** — egui / eframe (already in the shell).
3. **Raw serial** — both transparent byte pipe and delimited frames in the first build.
4. **MQTT** — client required; embedded broker / in-process server capability planned for hub scenarios (MQTT first; consider OPC UA for similar hub concepts). Some hub designs will use protocols already suited to that role.
5. **Modbus and OPC UA** — each ships **both client and server** in the first usable build.
6. **Platforms** — Linux **and** Windows in the first build.
7. **Mappings** — type-preserving copy only at first (no scale / endian / word-swap yet).
8. **Testing** — scripts → hub store → outputs, then a second hub importing those outputs; many-to-many and multi-hub with bi-directional protocols (see [Multi-hub test approach](#multi-hub-test-approach)).
10. **Modbus Bench** — Protocol Bench is a new, separate product; can be tested with Modbus Bench.
11. **File parsers** — CSV and JSON lines stay the first-build set.

## Still open

1. **Process shape** — one binary with optional headless mode, vs a service plus a separate bench UI. Still discussing; not decided.
9. **License / free vs paid isolation** — wants a free option that is **not easily reverse-engineered**. Preferred lean (not final): base product ships only a couple of simple free protocols; **added protocols come as separate file packs** customers buy and license. Design must also cover: **proper installer**, **auto-updater**, free/pro options, and a **central license server** (PC transfers, enterprise usage accounts). Closer to isolation options **B/D** (packs) plus a license server. Details: [free-vs-paid-isolation.md](free-vs-paid-isolation.md). Not a final product choice yet — needs further design. Do not implement packaging or license-server code until that design lands.
12. **Which protocols are free vs paid** — TBD (depends on #9).

## Polish still in front of us

- Live link state, quality, and last error on each endpoint.
- A saved project that restores endpoints and mappings (beyond window layout persistence).
- File follow byte offset across restart and rotated log.
- The rest of the protocol list, each on its own engine thread.
- Panel art that keeps following the theme as new diagrams are added.

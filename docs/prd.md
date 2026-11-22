# Protocol Bench — product requirements

Date: 2026-11-13. Reviewed against the code: 2026-11-19. Status: living draft. Build status and open items: [prd-gap-review.md](prd-gap-review.md).

Research: competitive landscape and protocol notes live in the research store (not in this repo). Free vs paid isolation: [free-vs-paid-isolation.md](free-vs-paid-isolation.md). Pack loading: [protocol-pack-loading.md](protocol-pack-loading.md).

Checkout: `/home/mike/Documents/AAI-Projects/protocolbench` — run instructions in [../README.md](../README.md). Workspace: `pb-abi`, `pb-points`, `pb-engine`, `pb-ipc`, `pb-mqtt`, `pb-opcua`, `pb-streams`, `pb-http`, `pb-service`, `pb-ui`.

## Product

Protocol Bench is a native Rust product for moving values between industrial and oilfield links. An always-on **service** holds the points and runs the converter; a separate **UI** configures and monitors it. A value that arrives on one protocol can leave on another. There is no dual-mode single-process bench (no “optional headless in the same binary”).

Live I/O runs on the engine threads in the service; all protocol packs in the Protocols list that have a panel are live (see [prd-gap-review.md](prd-gap-review.md) for what is not built). The window is egui (`eframe`), not a web UI. Interaction follows Modbus Bench, with a Protocol Bench look. UI ↔ service uses length-prefixed JSON over localhost (token in the config dir; see `pb-service/README.md`).

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
- Help → About: centered “Protocol Bench” wordmark, slogan **“Your protocol hub for every stream.”**, version, Agile Automation logo at the bottom.
- File menu: Open project… (native file dialog), Save project, Save project as…, Recent projects (up to 15), Quit. View menu: panel toggles, Show all panels, Reset layout.
- Status bar: left message + **service connection** chip (connected / disconnected over localhost IPC). The UI opens when the service is down and reconnects.
- Panels: Project, Hubs, Points, Mappings (read-only route list derived from hub Outputs), Traffic, Streams / TCP, Modbus, MQTT, OPC UA, File follow, HTTP / REST, LAS, SNMP. Each protocol panel owns **connections / endpoints**; hub Input / Output nodes own the binds.

## MCP server on the service

**Settled lean (Mike):** the always-on Protocol Bench **service** exposes an **MCP server** so an agent can manage setup — add / connect / test channels, points, mappings, hubs, and the rest of product functionality through MCP tools.

- Lives on the **service**, not in the UI process.
- Scope is product-wide configuration and operation the service already owns (channels, points, mappings, hubs, project setup, and related actions) — agent-driven setup alongside the separate config/monitor UI.
- Tool schema, transport, and auth are **not** specified yet; this is intent / direction only.

## Engines

Each protocol engine owns its work on a thread the UI does not block. `pb-engine` lives in the **service**; the UI takes a snapshot and submits commands (IPC details follow-on). Packs load via the C ABI (`pb-abi`) from a `modules/` directory. **MQTT client** is the first live pack (`pb-mqtt` / rumqttc): multi-client connect, wildcard subscribe, traffic topic tree, hub Input/Output point binds, publish. OPC UA, Streams, HTTP, Modbus, File follow, SNMP and LAS packs are also live.

## Hubs

A **hub** is the main protocol conversion / routing point. It is always **one protocol**, and only **server-capable** protocols can be hubs (Modbus server, MQTT broker / server-oriented, OPC UA server). **Input** (source) and **Output** nodes may use other protocols. Optional **aggregators** and **calculations** sit **on the edges** (the data path between nodes), not as free-floating peers of the hub. Flow: Inputs → optional transforms on edges → hub → optional transforms on edges → Outputs (and multi-hub as before). A diagram shows what converts to what.

- More than one hub when there are multiple conversion points. Many projects use a single hub. There is no Split hub type.
- Arrange the diagram: drag nodes, click a node or connection for details, open the matching protocol panel from a protocol node.
- Add aggregator / Add calculation attaches to a **selected link/edge** (or a right-clicked connection), rendered on that line.
- Sample hubs ship for framing (Plant: Modbus server hub with a Tank PLC input, a Power meter RTU input, MQTT publishes and a pump-command Modbus output; Edge: MQTT broker hub) — configuration only; the sample's Modbus connections are disabled until enabled. A runnable version lives in the demo project (see README).

### Calculations vs Aggregators

| Kind | Role |
| --- | --- |
| **Calculation** | **Single-stream** edge transform: one source point → one output on that path (scale/offset, deadband, rolling average, rate of change, hysteresis, …). Never combines multiple points. |
| **Aggregator** | **N≥2 member points** combined into one derived host point `agg_N`. Modes: sum, mean, min, max, count, median, first, last, any-good, majority. Optional window: none / last N samples / last T seconds. Member **quality is worst-wins**. |

Calculations must not grow multi-input pickers — multi-point combine belongs only on Aggregator nodes. Aggregator config persists under each hub’s `aggregators[]` in `project.json`.

### MQTT model (client pack)

| Surface | Owns |
| --- | --- |
| **MQTT panel** | Connections only: broker host/port, client id (optional Unique), credentials, transport, Connect/Disconnect, optional **wide subscribe** (`+` / `#` wildcards OK) for traffic / bring-up |
| **Hub Input node** | Point promotion: pick a shared connection + topic/field → auto sequential `mqtt_N` (no manual point-id picker). Click leaf in topic tree to bind |
| **Hub Output node** | Publish: pick connection + topic + which points to publish |

Rules:

- Many **Input** nodes may **share one MQTT connection**; each owns its own topic/field → point binds. Overlap across nodes is OK (different graph uses).
- Wide subscribe may be broad; **only hub Input–defined paths become points** (pack traffic still feeds the topic tree).
- Shared **topic tree** (TopicBench-style): MQTT panel preview + Input inspector. Tree is fed from pack `traffic` snapshot, merged with live point sources so it stays aligned with the Points panel.
- MQTT Input diagram nodes show bound **point count** on the right of the title. Input inspector has no Hide/Remove — delete via diagram later.
- Persist in `project.json` (current version 10): `mqtt.clients[]`, `mqtt.embedded` (hub broker listen; rumqttd Start/Stop in MQTT panel), `hubs` (binds), `points[]`, `next_point_seq`; other protocols add `opcua`, `http`, `streams`, `las`, `snmp`, `modbus`, `files`.

### Modbus model (live: TCP + RTU, client + server)

| Surface | Owns |
| --- | --- |
| **Modbus panel — Client connection** | A remote device: TCP host:port or RTU serial line, default unit, timeout, retries |
| **Modbus panel — Server connection** | A Modbus server this product hosts (TCP listen or RTU slave) **and its own register map**: which host point is served at which area / address / type / order. Set up independently of client connections |
| **Hub Input node** | Pick a Client connection + poll interval; bind registers (area, address, type, byte order, optional unit override) → auto `modbus_N` |
| **Hub Output node** | Pick a Client connection; bind a host point → coil / holding register (write) |
| **Modbus hub** | Picks **one** Server connection to listen on |

Data types per bind: `bool` (coil / discrete), `u16 i16 u32 i32 u64 i64 f32 f64`, `bit0..15` (one bit of a register), `strN` (ASCII over N registers). Multi-byte types carry a **byte / word order** — `abcd` (big-endian), `cdab` (word swap), `badc` (byte swap), `dcba` (little-endian). Type-preserving only; no scaling. The register codec lives in `pb-modbus` (`codec.rs`, unit-tested). The `pb-modbus` pack runs one worker per Client connection (batched polling, bad quality + reconnect backoff, retries, typed writes) and serves each Server connection a register image mirrored with host points: host point changes update the image, master writes update the points. A Server connection runs only when a Modbus hub picks it. Unmapped addresses return exception 2. TCP servers answer the configured unit and 255; RTU slaves answer the configured unit and accept unit-0 broadcast writes.

### SNMP model (live: v1 / v2c GET)

**SNMP panel** owns targets (host, port, version, community, timeout); a hub **SNMP Input** picks a target + poll interval and binds OIDs → `snmp_N`. The `pb-snmp` pack has its own BER codec (known-byte tested) and runs one worker per target: requests are batched (10 varbinds, halved on `tooBig`), retried, and per-OID failures (noSuchInstance, v1 noSuchName) mark only that point bad. Counter / Gauge / TimeTicks / Counter64 become integers (floats above i64), OctetStrings text (hex when not printable), IpAddress / OID text. An agent that stops answering turns its points bad (last value held) and is retried; the socket is reopened. **Not yet:** SNMPv3, SET Output, walks / traps.

### LAS model (live: LAS 1.2 / 2.0, unwrapped)

**LAS panel** owns endpoints (path, role incoming / outgoing, follow); a hub **LAS Input** binds curve mnemonics → `las_N`, a hub **LAS Output** binds host points → curves with an index of `depth` or `time`.
- **Incoming:** the `pb-las` pack follows the file with the shared byte-offset tailer (`pb-tail`): header sections `~V ~W ~C ~A` are parsed, rows are whitespace-separated, the file's `NULL` value makes that curve bad (last value held), and the offset survives restarts and rotation like File follow. `follow = off` loads the file once. `WRAP YES` (multi-line rows) is refused with a clear error.
- **Outgoing:** a depth-indexed output (needs a `DEPT` bind whose point is the depth) records one row each time the depth changes, using the latest value of every other curve (NULL until a value arrives); a time-indexed output writes at most one row per second with epoch-seconds as `TIME`. A missing / empty file gets a fresh LAS 2.0 header; an existing file is only appended to when its curves match exactly, otherwise nothing is written and the error says why. `STOP` in the header is not updated while logging.
- **Curve picker:** an incoming endpoint with no binds still lists the file's curves in the hub inspector (preview mode).

### File follow model (live: CSV + JSON lines)

**File follow panel** owns sources (path, CSV or JSON-lines parser, delimiter / header row, poll, start at beginning or end, rotation handling). A hub **Files Input** picks a source and binds CSV column / JSON path → `file_N`. The `pb-files` pack runs one worker per followed source: it reads only **complete lines** past a saved byte offset (a half-written row waits for its newline), keeps that offset on disk (`<config dir>/file-follow/<source>.json`, with the file's identity) so a restart resumes where it stopped — rows written while the service was down are caught up, seen rows are never re-read — and restarts from byte 0 with a fresh header when the file is truncated or replaced (log rotation). Cells are typed (bool / integer / float / text); JSON lines use dotted paths with `[n]` indices. A missing file turns its points bad (last value held) and recovers when it reappears. **Column picker:** the pack reports the header names (or JSON leaf paths) with the latest row's values — even for a source with no binds yet (preview mode) — and the hub Files Input shows them as clickable chips, so it also works when the service runs on another machine. One physical line = one row (no multi-line quoted CSV fields); the path is read on the machine running the service.

## What runs today, and what does not

Today: the window, layout, themes, the hub diagram with Input/Output binds for every protocol, the engine thread boundary, host point store (`pb-points`), pack loader, headless `pb-service` with localhost IPC, and live packs for **MQTT** (client + embedded broker; TCP / TLS / WS / WSS), **OPC UA** (client + server), **Streams** (serial / TCP), **HTTP / REST**, **Modbus** (TCP / RTU, client + server), **File follow** (CSV / JSON lines), **SNMP** (v1 / v2c GET) and **LAS** (2.0 follow + write).

Not yet: Sparkplug, DNP3, WebSocket, CAN, WITS, license signing enforcement, MSI, MCP server. Embedded MQTT broker (rumqttd) Start/Stop is live in the MQTT panel. Project open/save uses a native file requester (`rfd`) on a worker thread so the egui loop never blocks (Hyprland stalls if the dialog runs on the UI thread).

## Protocols

Already named in the shell:

- Raw serial (Streams) — first build: **both** transparent byte pipe and delimited frames
- Modbus — first build: **client and server**
- MQTT — **client pack** and **embedded broker** (rumqttd) both live
- OPC UA — first build: **client and server**
- File follow — first build parsers: **CSV and JSON lines**

Also in the product (HTTP, SNMP and LAS have panels; the rest do not yet):

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

Short version (detail in the landscape doc): Neuron, Kepware, and DataHub are plant connectivity servers; HighByte and HiveMQ Edge assume OPC/MQTT-shaped data; Node-RED / Telegraf / PLC4X are glue. Protocol Bench takes serial + Modbus + MQTT + OPC UA + file follow (and oilfield LAS/WITS) as a native Rust always-on service with a separate config/monitor UI, packs loaded in the service.

## Runtime

- Language: **Rust** for the Protocol Bench **service** and **UI** (workspace today: `pb-ui` egui/eframe, `pb-engine`; service/UI split is the settled process shape — see Settled #1).
- Online **license server** can also be Rust. Cloudflare Workers supports Rust via [workers-rs](https://github.com/cloudflare/workers-rs) (first-class language). That fit is viable for a license API on Workers with D1 / KV / R2. Workers is not a long-running tokio process — use Containers or a VPS if that model is required. Hosting is not over-specified here; CF Workers + Rust is noted as a viable option only.
- Candidate crates (spikes, not defaults): `serialport`, `tokio-modbus`, `rumqttc` (or equivalent), [async-opcua](https://github.com/FreeOpcUa/async-opcua) (MPL-2.0 subset).
- Adjacent product: Modbus Bench remains Modbus-only; Protocol Bench is separate and can be tested with it.

## Settled decisions

Recorded 2026-11-13 from Mike’s answers (numbering matches the former Open decisions list):

1. **Process shape** — **service always + separate UI**. The service is the always-on converter; the UI is for config and monitoring only. No dual-mode single-process bench. Protocol packs load in the **service**. Local IPC between UI and service is follow-on design.
2. **UI toolkit** — egui / eframe (already in the shell).
3. **Raw serial** — both transparent byte pipe and delimited frames in the first build.
4. **MQTT** — client required; embedded broker / in-process server capability planned for hub scenarios (MQTT first; consider OPC UA for similar hub concepts). Some hub designs will use protocols already suited to that role.
5. **Modbus and OPC UA** — each ships **both client and server** in the first usable build.
6. **Platforms** — Linux **and** Windows in the first build.
7. **Mappings** — type-preserving copy only at first (no scale / endian / word-swap yet).
8. **Testing** — scripts → hub store → outputs, then a second hub importing those outputs; many-to-many and multi-hub with bi-directional protocols (see [Multi-hub test approach](#multi-hub-test-approach)).
9. **Pack architecture + entitlements** — core loads **signed protocol packs** via a **stable C-style plugin ABI** into `pb-engine` (in-process `cdylib` packs) inside the **service**. Packs download from the **license server** after purchase / entitlement. **No fixed free protocol list** — the license is entitlement-driven and defines which protocols/packs the customer gets. Offline grace is about **~7 days** (phone-home to the license server; not a strict check on every operation). Paid protocol source stays in packs, not in the free/core binary. This is the settled product path for licensing and protocol source management — **not** sidecar processes, **not** an all-in-one license-gated binary. Details: [free-vs-paid-isolation.md](free-vs-paid-isolation.md), [protocol-pack-loading.md](protocol-pack-loading.md).
10. **Modbus Bench** — Protocol Bench is a new, separate product; can be tested with Modbus Bench.
11. **File parsers** — CSV and JSON lines stay the first-build set.
12. **Language** — Protocol Bench service + UI remain Rust. Online license server can also be Rust; Cloudflare Workers + workers-rs is a viable host for a license API (see [Runtime](#runtime)).
13. **Windows installer** — **MSI via WiX**, driven from the Rust repo with **`cargo-wix`** (not Inno Setup). Chosen for IT-friendly deploys (GPO / SCCM / silent install, standard Add/Remove Programs). Layout installs **service + UI + module dir**; upgrades must **preserve config, licenses, and packs**. Product **updates** stay on the Rust **license server** path (manifest + signed downloads) — the installer choice does not replace that. **Linux installer** is later / TBD and is not blocking.
14. **MCP on the service** — the always-on **service** hosts an **MCP server** so an agent can manage setup (channels, points, mappings, hubs, and related product actions). Not in the UI process. High-level product intent only; no tool schema yet (see [MCP server on the service](#mcp-server-on-the-service)).
15. **Hub graph model** — **Hub** = conversion / routing point (multiple hubs allowed for multiple conversion points). **Aggregator** and **Calculation** are edge transforms on the data path between nodes (sources → optional edge transforms → hub → optional edge transforms → outputs), not independent sibling hub-step nodes. UI adds them on a selected / right-clicked connection. **Calculation** = single-stream; **Aggregator** = N≥2 member combine → `agg_N` (see [Calculations vs Aggregators](#calculations-vs-aggregators)).
16. **Hub protocol** — every hub is **one protocol**; **Split** hubs are removed. Only **server-capable** protocols may be hubs (Modbus server, MQTT broker/server-oriented, OPC UA server). Sources and outputs define other protocols on the diagram.

## Still open

9. **License / packs — remaining details** — architecture, entitlements, ~7-day offline grace, and the Windows MSI/`cargo-wix` installer are settled (see Settled #1, #9, #12, #13). Still open: **auto-updater** poll / UX details, **Linux** install channel (later / TBD), and any further license-server product details beyond the Runtime note. Do not implement packaging or license-server code until those residual pieces land.

## Polish still in front of us

- Live link state, quality, and last error on each endpoint.
- A saved project that restores endpoints and mappings (beyond window layout persistence).
- File follow byte offset across restart and rotated log.
- The rest of the protocol list, each on its own engine thread.
- Panel art that keeps following the theme as new diagrams are added.

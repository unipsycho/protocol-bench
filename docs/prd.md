# Protocol Bench — product requirements

Date: 2026-11-13. Status: living draft.

Research: [competitive-landscape.md](competitive-landscape.md) (store). Free vs paid isolation: [free-vs-paid-isolation.md](free-vs-paid-isolation.md). Pack loading: [protocol-pack-loading.md](protocol-pack-loading.md). Protocol notes: [protocol-options.md](protocol-options.md).

Checkout: `/home/mike/Documents/AAI-Projects/protocol bench` (workspace `pb-ui` + `pb-engine`, egui shell).

## Product

Protocol Bench is a native Rust product for moving values between industrial and oilfield links. An always-on **service** holds the points and runs the converter; a separate **UI** configures and monitors it. A value that arrives on one protocol can leave on another. There is no dual-mode single-process bench (no “optional headless in the same binary”).

This build is still the shell. Live protocol I/O comes later, on the engine threads in the service. The window is egui (`eframe`), not a web UI. Interaction follows Modbus Bench, with a Protocol Bench look. Local IPC between UI and service is follow-on design (not specified here).

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
- File menu: Open project, Save as project, Quit. View menu: panel toggles, Show all panels, Reset layout.
- Status bar: left message + mock **service connection** (connected / disconnected) for the future always-on service — not a pb-engine host readout.
- Panels: Project, Hubs, Points, Mappings, Traffic, Streams, Modbus, MQTT, OPC UA, File follow.

## MCP server on the service

**Settled lean (Mike):** the always-on Protocol Bench **service** exposes an **MCP server** so an agent can manage setup — add / connect / test channels, points, mappings, hubs, and the rest of product functionality through MCP tools.

- Lives on the **service**, not in the UI process.
- Scope is product-wide configuration and operation the service already owns (channels, points, mappings, hubs, project setup, and related actions) — agent-driven setup alongside the separate config/monitor UI.
- Tool schema, transport, and auth are **not** specified yet; this is intent / direction only.

## Engines

Each protocol engine owns its work on a thread the UI does not block. `pb-engine` lives in the **service**; the UI takes a snapshot and submits commands (IPC details follow-on). The five slots that exist now (Streams, Modbus, MQTT, OPC UA, File follow) are idle — they do not open a serial port, a socket, or a file.

## Hubs

A **hub** is the main protocol conversion / routing point. It is always **one protocol**, and only **server-capable** protocols can be hubs (Modbus server, MQTT broker / server-oriented, OPC UA server). Sources and outputs may use other protocols. Optional **aggregators** and **calculations** sit **on the edges** (the data path between nodes), not as free-floating peers of the hub. Flow: sources → optional transforms on edges → hub → optional transforms on edges → outputs (and multi-hub as before). A diagram shows what converts to what.

- More than one hub when there are multiple conversion points. Many projects use a single hub. There is no Split hub type.
- Arrange the diagram: drag nodes, click a node or connection for details, open the matching protocol panel from a protocol node.
- Add aggregator / Add calculation attaches to a **selected link/edge** (or a right-clicked connection), rendered on that line.
- Sample hubs ship for framing (Plant Modbus + Edge MQTT, MQTT link between them) — configuration only, not a live bridge.

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
15. **Hub graph model** — **Hub** = conversion / routing point (multiple hubs allowed for multiple conversion points). **Aggregator** and **Calculation** are edge transforms on the data path between nodes (sources → optional edge transforms → hub → optional edge transforms → outputs), not independent sibling hub-step nodes. UI adds them on a selected / right-clicked connection.
16. **Hub protocol** — every hub is **one protocol**; **Split** hubs are removed. Only **server-capable** protocols may be hubs (Modbus server, MQTT broker/server-oriented, OPC UA server). Sources and outputs define other protocols on the diagram.

## Still open

9. **License / packs — remaining details** — architecture, entitlements, ~7-day offline grace, and the Windows MSI/`cargo-wix` installer are settled (see Settled #1, #9, #12, #13). Still open: **auto-updater** poll / UX details, **Linux** install channel (later / TBD), and any further license-server product details beyond the Runtime note. Do not implement packaging or license-server code until those residual pieces land.

## Polish still in front of us

- Live link state, quality, and last error on each endpoint.
- A saved project that restores endpoints and mappings (beyond window layout persistence).
- File follow byte offset across restart and rotated log.
- The rest of the protocol list, each on its own engine thread.
- Panel art that keeps following the theme as new diagrams are added.

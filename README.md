# Protocol Bench

Native Rust product for moving values between industrial and oilfield links (serial / TCP streams, Modbus, MQTT, OPC UA, HTTP / REST, SNMP, LAS, CSV / JSON-lines file follow, WITS, WebSocket, Sparkplug B, DNP3, CAN, plus a Generator source that makes up data). An always-on **service** (`pb-service`) holds the points and runs the protocol packs; a separate **UI** (`protocol-bench`, egui / eframe) configures and monitors it.

> Your protocol hub for every stream. — Agile Automation

Product requirements: [docs/prd.md](docs/prd.md). Review / open items: [docs/prd-gap-review.md](docs/prd-gap-review.md).

## Requirements

- Rust (stable, edition 2021) with `cargo` — <https://rustup.rs>
- Linux (first target) or Windows. On Linux the UI needs a working Wayland or X11 session plus the usual egui/`rfd` libraries (`libxkbcommon`, `libwayland`, `libGL`; an XDG desktop portal or GTK for file dialogs).
- No external services are required. MQTT, OPC UA and Modbus peers are optional (Modbus Bench is the intended far end for testing).

## Run

Start the service (with the protocol packs it loads), then the UI:

```bash
# 1. Build the packs and the service, and put the packs in modules/
cargo build -p pb-mqtt -p pb-opcua -p pb-streams -p pb-http -p pb-modbus -p pb-files -p pb-snmp -p pb-las \
            -p pb-wits -p pb-websocket -p pb-sparkplug -p pb-dnp3 -p pb-can -p pb-gen -p pb-service
mkdir -p modules && cp target/debug/libpb_*.so modules/     # pb_*.dll on Windows

# 2. Service (always-on; owns project.json, serves localhost JSON IPC)
PB_MODULES=$PWD/modules cargo run -p pb-service

# 3. UI (default workspace member — a plain `cargo run` starts it)
cargo run
```

**Shortcut:** `scripts/dev.sh` does all of the above in one go: it stops any running service / UI, rebuilds, refreshes `modules/`, starts the service and then the UI. Closing the UI stops the service. Options: `--release`, `--project FILE`, `--no-ui`.

Keep the service running in its own terminal (it stays in the foreground and prints `IPC listening on 127.0.0.1:<port>`); the UI goes in a second terminal. The UI opens even when the service is down and reconnects automatically (about every 1.5 s, re-reading `~/.config/protocolbench/ipc.json`), so if the status bar says "Service disconnected", check that the service process is still alive (`pgrep -a pb-service`). Release build: `cargo build --release`.

### Environment

| Variable | Meaning |
| --- | --- |
| `PB_MODULES` | Directory the service loads protocol packs from (unset = no packs loaded) |
| `PB_PROJECT` | Project file the service opens (otherwise the project it had open last — `~/.config/protocolbench/last-project` — or the default `project.json`) |
| `PB_IPC_PORT` | Fixed localhost IPC port (otherwise chosen and written to `~/.config/protocolbench/ipc.json` with `ipc.token`) |
| `PB_NO_TRAY` | `1` = no status-area tray icon (tests, headless runs) |
| `PB_AUTO_RUN_HUBS` | `1` = start the project's embedded MQTT broker / OPC UA server at service start, with no UI attached (headless demos) |

### Tests

```bash
scripts/test-all.sh             # everything below, one command (add --quick for unit tests only)

cargo test --workspace         # includes pb-service/tests: real service binary in a temp config dir (restart, IPC contract)
./scripts/modbus-smoke.sh       # service + Modbus pack (client + server) against a fake TCP device
./scripts/modbus-rtu-smoke.sh   # Modbus RTU master + slave over a virtual serial pair (pty), no hardware
./scripts/files-smoke.sh        # File follow: appends, restart catch-up, rotation, column picker
./scripts/snmp-las-smoke.sh     # fake SNMP agent + LAS file follow / write
./scripts/demo-smoke.sh         # pb-sim devices → every pack → the three industrial demos, plus the generator demo (no devices; checks points, MQTT topics, moving data)
./scripts/pb-service-smoke.sh   # IPC smoke (also mqtt-smoke.sh, opcua-smoke.sh)
```

The smokes need python3 and use a private config dir, so a running UI never attaches to them. (The older `pb-service-smoke.sh` / `mqtt-smoke.sh` / `opcua-smoke.sh` use the shared config — close the UI first.)

## Workspace

| Crate | Role |
| --- | --- |
| `pb-ui` | egui desktop shell: dock panels, hub diagram, project open / save (binary `protocol-bench`) |
| `pb-service` | Always-on host: loads packs, owns `project.json`, IPC server |
| `pb-engine` | Engine host, calculations, aggregators, pack loader |
| `pb-points` | Host point store (short ids ↔ wire addresses, quality) and `addr`, the single definition of every wire-address format |
| `pb-ipc` | UI ↔ service framing and client |
| `pb-abi` | Stable C ABI shared by the host and packs, plus the `Pack` trait / `export_pack!` macro that packs implement |
| `pb-mqtt`, `pb-opcua`, `pb-streams`, `pb-http`, `pb-modbus`, `pb-files`, `pb-snmp`, `pb-las` | Protocol packs (`cdylib`) — MQTT client + embedded broker, OPC UA client + server, serial / TCP streams, HTTP / REST, Modbus TCP / RTU client + server (register maps with scaling), File follow (CSV / JSON lines), SNMP v1 / v2c GET, LAS 2.0 follow + write |
| `pb-wits`, `pb-websocket`, `pb-sparkplug`, `pb-dnp3`, `pb-can` | Ext protocol packs (`cdylib`) — WITS level 0 in / out (TCP / serial), WebSocket JSON client + server, Sparkplug B host + edge node, DNP3 TCP master (polls + controls), CAN / SocketCAN signals (J1939 PGNs) |
| `pb-gen` | Generator pack (`cdylib`) — synthetic signals (waveforms, random walks, process shapes, counters, formulas) on interval / random-timer / burst triggers; a source only. Models live in the plain rlib `pb-signal`, the expression language in `pb-expr` (also behind Calculation-node formulas) |
| `pb-extkit` | Shared runtime for the ext packs: workers per connection, config diffing, output routing, status / "seen on the wire" |
| `pb-proto` | Pure codecs shared by packs and the simulator: WITS, Sparkplug B protobuf, DNP3 (link / transport / application + demo outstation), CAN signals + SocketCAN |
| `pb-sim` | Demo field devices (frac spread, bottling line, lift station) and `watch` / `check` point tools |
| `pb-tail` | Shared byte-offset file tailing (rotation detection, saved state) used by the File follow and LAS packs |
| `pb-json` | One JSON path syntax (`a.b`, `a[0]`, `$.a`) shared by the MQTT, HTTP and File follow packs |

All protocol packs in the table are live. The service also hosts an MCP server for agents (see `pb-service/README.md`). See the gap review for what is not built.

## Key dependencies

`eframe` / `egui` 0.29 (UI), `rumqttc` + `rumqttd` (MQTT client / broker), `async-opcua` (OPC UA), `serialport` (serial), `ureq` (HTTP), `tokio`, `serde` / `serde_json`, `libloading` (pack loading), `rfd` (native file dialogs). Versions are pinned in [Cargo.toml](Cargo.toml).

## Try the Modbus Plant demo

```bash
python3 scripts/modbus-plant-sim.py            # fake tank PLC on 127.0.0.1:5020 (unit 1)
PB_MODULES=$PWD/modules PB_PROJECT=../protocol-bench-projects/modbus-plant.json cargo run -p pb-service
cargo run                                       # UI — Hubs, Modbus and Points panels show live values
```

The Plant hub polls the PLC, publishes to MQTT, serves the same points as a Modbus server on `:1502`, and a `plant/pump/cmd` MQTT message drives the PLC's pump coil. The sample project shipped in a fresh install has its Modbus connections **disabled**; the demo file enables them.

More samples (`modbus-hub.json`, `edge-logger.json`, with `scripts/field-sim.py` as their SNMP agent and file feeder) are described in `../protocol-bench-projects/README.md`.

> The UI and service find each other through `~/.config/protocolbench/ipc.json`. Only one service should write it at a time — an open UI attaches to whichever service wrote it last.

## Industrial demos (all Rust)

```bash
cargo run -p pb-sim -- all                      # every simulated device (frac, bottling, water)
scripts/dev.sh --project ../protocol-bench-projects/frac-spread.json
```

| Project | What it shows |
| --- | --- |
| `frac-spread.json` | Frac pumps + blender (Modbus TCP), wellhead RTU (DNP3), pressure-pumping WITS feed, pump engine (CAN J1939) → WITS to the data van, WebSocket dashboard `:8765`, MQTT `frac/#`, stage LAS log, ESD and rate commands back to the devices |
| `bottling-line.json` | Line PLC, VFDs, energy meter (Modbus TCP), robot cell (Sparkplug B edge) → ISA-95 UNS, Sparkplug gateway edge node, WebSocket dashboard `:8766`, run / speed commands |
| `water-lift-station.json` | Lift station RTU (DNP3), chlorine analyzer (Modbus) → OPC UA server hub `:4855`, two-way web HMI on WebSocket `:8767` |

Headless (web demos): `PB_AUTO_RUN_HUBS=1 PB_MODULES=$PWD/modules PB_PROJECT=… pb-service` runs a project with its embedded broker / OPC UA server and no UI. `pb-sim watch` prints the live point table.

## Projects

A project is one JSON file (current version 11). File → New project creates an empty one. Sample projects live in `../protocol-bench-projects/`. UI layout, theme and recent projects are stored separately in `~/.config/protocolbench/ui.toml`; the service remembers the open project in `last-project` and reopens it after a restart or crash.

## More docs

- [docs/free-vs-paid-isolation.md](docs/free-vs-paid-isolation.md) — licensing / entitlement split
- [docs/protocol-pack-loading.md](docs/protocol-pack-loading.md) — pack ABI and loading
- [docs/performance.md](docs/performance.md) — CPU / RAM measurements and how to re-run them
- [docs/pack-development.md](docs/pack-development.md) — writing a pack (`pb_abi::Pack`), contracts that are not obvious from the code, known quirks
- [pb-service/README.md](pb-service/README.md) — service details and IPC methods

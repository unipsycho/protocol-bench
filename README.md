# Protocol Bench

Native Rust product for moving values between industrial and oilfield links (serial / TCP streams, Modbus, MQTT, OPC UA, HTTP / REST, SNMP, LAS, CSV / JSON-lines file follow). An always-on **service** (`pb-service`) holds the points and runs the protocol packs; a separate **UI** (`protocol-bench`, egui / eframe) configures and monitors it.

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
cargo build -p pb-mqtt -p pb-opcua -p pb-streams -p pb-http -p pb-modbus -p pb-files -p pb-snmp -p pb-las -p pb-service
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
| `PB_PROJECT` | Project file the service opens (otherwise the default project path) |
| `PB_IPC_PORT` | Fixed localhost IPC port (otherwise chosen and written to `~/.config/protocolbench/ipc.json` with `ipc.token`) |

### Tests

```bash
scripts/test-all.sh             # everything below, one command (add --quick for unit tests only)

cargo test --workspace
./scripts/modbus-smoke.sh       # service + Modbus pack (client + server) against a fake TCP device
./scripts/modbus-rtu-smoke.sh   # Modbus RTU master + slave over a virtual serial pair (pty), no hardware
./scripts/files-smoke.sh        # File follow: appends, restart catch-up, rotation, column picker
./scripts/snmp-las-smoke.sh     # fake SNMP agent + LAS file follow / write
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
| `pb-mqtt`, `pb-opcua`, `pb-streams`, `pb-http`, `pb-modbus`, `pb-files`, `pb-snmp`, `pb-las` | Protocol packs (`cdylib`) — MQTT client + embedded broker, OPC UA client + server, serial / TCP streams, HTTP / REST, Modbus TCP / RTU client + server, File follow (CSV / JSON lines), SNMP v1 / v2c GET, LAS 2.0 follow + write |
| `pb-tail` | Shared byte-offset file tailing (rotation detection, saved state) used by the File follow and LAS packs |
| `pb-json` | One JSON path syntax (`a.b`, `a[0]`, `$.a`) shared by the MQTT, HTTP and File follow packs |

All protocol packs in the table are live. See the gap review for what is not built (Sparkplug B, DNP3, WebSocket, CAN, WITS, MCP server).

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

## Projects

A project is one JSON file (current version 10). Sample projects live in `../protocol-bench-projects/`. UI layout and theme are stored separately in `~/.config/protocolbench/ui.toml`.

## More docs

- [docs/free-vs-paid-isolation.md](docs/free-vs-paid-isolation.md) — licensing / entitlement split
- [docs/protocol-pack-loading.md](docs/protocol-pack-loading.md) — pack ABI and loading
- [docs/pack-development.md](docs/pack-development.md) — writing a pack (`pb_abi::Pack`), contracts that are not obvious from the code, known quirks
- [pb-service/README.md](pb-service/README.md) — service details and IPC methods

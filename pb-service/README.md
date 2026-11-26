# pb-service

Always-on Protocol Bench host: loads protocol packs, owns `project.json`, serves **localhost JSON IPC**.

## Linux (first)

```bash
cargo build -p pb-mqtt -p pb-opcua -p pb-streams -p pb-http -p pb-modbus -p pb-files -p pb-snmp -p pb-las -p pb-service
mkdir -p modules && cp target/debug/libpb_*.so modules/
PB_MODULES=$PWD/modules cargo run -p pb-service
# optional: PB_IPC_PORT=17500 PB_PROJECT=/path/to/project.json
```

Writes `~/.config/protocolbench/ipc.json` + `ipc.token`. UI connects with those files (no in-process engine).

Smoke: `./scripts/pb-service-smoke.sh`

## Windows Service

Ship shape is a true Windows Service (`--windows-service` entry scaffolded). Full SCM/`windows-service` crate dispatcher and MSI wiring follow after Linux IPC is solid.

## IPC

Length-prefixed JSON (`u32` LE + body). Methods: `hello`, `ping`, `status`, `snapshot`, `submit`, `set_calcs`, `set_aggs`, `get_project`, `put_project`, `open_project`, `stop_all`, `import_bindings`.

`open_project` stops every embedded hub and client the previous project started before the new project's config is applied. `stop_all` does the same without loading anything (project file untouched); the UI's "Stop all and quit" and the tray's "Stop all hubs" use it.

## Project lifecycle

- The service holds one active project. A starting UI **attaches** to it (`status.project_path` + `get_project`) instead of re-opening the default, so what is running is what you see. A service with nothing live is asked to open the project so auto-run still applies.
- Closing the UI leaves everything running. The UI warns on quit (window close, Alt+F4 or File > Quit) when hubs are live and offers **Quit, keep running**, **Stop all and quit** or **Stay**.
- Switching projects in the UI lists what will stop and asks first.

## Tray (Linux)

A StatusNotifierItem (D-Bus, no GTK) shows the Protocol Bench mark with a green dot when hubs are live and grey when idle. Tooltip and menu list the project and each live hub; the menu has **Stop all hubs** and **Quit service**. Other platforms run without a tray for now.


## MCP server (agents)

The service also serves the Model Context Protocol (Streamable HTTP, `POST /mcp`, JSON responses) on **127.0.0.1** so an AI agent can inspect and set up Protocol Bench (PRD settled #14).

| Variable | Meaning |
| --- | --- |
| `PB_MCP_PORT` | Fixed MCP port (otherwise any free port; the URL is written to `<config dir>/mcp.json`) |
| `PB_MCP=off` | Do not start the MCP server |
| `PB_AUTO_RUN_HUBS=1` | Start embedded brokers / OPC UA servers at service start (headless demos) |

Every request needs `Authorization: Bearer <ipc.token>`. Tools: `get_status`, `list_protocols`, `list_points`, `read_point`, `write_point`, `list_hubs`, `get_project`, `set_project_section`, `add_connection`, `open_project`, `stop_all_hubs` (see `src/mcp.rs`). Edits are applied and saved immediately; an open UI sees them after the project is reopened.

```bash
claude mcp add --transport http protocol-bench http://127.0.0.1:7741/mcp \
  --header "Authorization: Bearer $(cat ~/.config/protocolbench/ipc.token)"
```

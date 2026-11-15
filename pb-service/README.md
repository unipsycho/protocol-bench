# pb-service

Always-on Protocol Bench host: loads protocol packs, owns `project.json`, serves **localhost JSON IPC**.

## Linux (first)

```bash
cargo build -p pb-mqtt -p pb-opcua -p pb-service
mkdir -p modules && cp target/debug/libpb_*.so modules/
PB_MODULES=$PWD/modules cargo run -p pb-service
# optional: PB_IPC_PORT=17500 PB_PROJECT=/path/to/project.json
```

Writes `~/.config/protocolbench/ipc.json` + `ipc.token`. UI connects with those files (no in-process engine).

Smoke: `./scripts/pb-service-smoke.sh`

## Windows Service

Ship shape is a true Windows Service (`--windows-service` entry scaffolded). Full SCM/`windows-service` crate dispatcher and MSI wiring follow after Linux IPC is solid.

## IPC

Length-prefixed JSON (`u32` LE + body). Methods: `hello`, `ping`, `status`, `snapshot`, `submit`, `set_calcs`, `set_aggs`, `get_project`, `put_project`, `open_project`, `import_bindings`.

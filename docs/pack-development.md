# Writing and maintaining a protocol pack

How the thirteen in-tree packs (`pb-mqtt`, `pb-opcua`, `pb-streams`, `pb-http`, `pb-modbus`, `pb-files`, `pb-snmp`, `pb-las`, and the ext packs `pb-wits`, `pb-websocket`, `pb-sparkplug`, `pb-dnp3`, `pb-can`) are built, and the rules that are not obvious from reading one of them. Background on *why* packs are `cdylib`s loaded by the service is in [protocol-pack-loading.md](protocol-pack-loading.md).

## Shape of a pack

```
pb-<name>/
  Cargo.toml        crate-type = ["cdylib", "rlib"]   (rlib so the crate's own tests can link it)
  src/lib.rs        protocol code
  src/pack.rs       PackState + `impl pb_abi::Pack` + `pb_abi::export_pack!(PackState);`
```

`pb_abi::Pack` ([pb-abi/src/pack.rs](../pb-abi/src/pack.rs)) is the whole contract. The macro generates the C entry points (`pb_pack_abi_version`, `pb_pack_register`) and all the pointer handling, so a pack contains **no `unsafe`** of its own.

| You implement | The host calls it when |
| --- | --- |
| `new(host)` | the pack is loaded; keep the [`HostRef`] — it is `Copy + Send + Sync` and is how workers publish points and log |
| `configure(json)` | the service (re)sends config — on project open, on every edit. **Must be idempotent and cheap when nothing changed**: keep running workers whose config is unchanged (see the worker `key` in `pb-snmp` / `pb-files` / `pb-las` / `pb-modbus`) |
| `config_failed(err)` | `configure` returned `Err`; record it so `last_error` shows it |
| `set_enabled(on)` / `connect` / `disconnect` | Start / Stop. `set_enabled` is enough for worker-thread packs |
| `shutdown()` | the pack is being destroyed — **join every worker thread here** (and in `Drop`). The host frees the callback context right after |
| `write_point(name, kind, bytes)` | egress: a host point changed and some Output may want it |
| `poll()` | every engine tick (~100 ms); only for packs that cannot push (`pb-http`) or that hand events across a channel (`pb-mqtt`, `pb-opcua`) |
| `state_label` / `last_error` / `is_connected` | status chips; keep them cheap, they run every tick |
| `detail_json()` | pack-specific status; the UI parses it from the slot's `detail_json` (connection tables, column pickers, …) |

Set `const SUBSCRIBE` / `PUBLISH` only if the pack really has those entry points — the host treats a missing one differently from one that always fails.

## Rules that bite

**Quality, not silence.** When a device stops answering, publish the last value again with `PB_QUALITY_BAD` (held value + bad quality) and keep retrying; when it returns, publish good. Never stop publishing and never publish a made-up value. Parse failures of one field mark only that point bad.

**Egress sends empty strings.** The engine calls `write_point` for *every* changed point on every connected pack, including points that have no value yet (empty string). Ignore a blank value unless the target type is a string (Modbus does; see `blank_point_values_are_ignored_not_errors`). Packs that are read-only return `0` and do nothing (the default).

**Point names and sources.** A pack upserts `(name, source)`. `name` is the short id the host assigned (`modbus_3`) or, when the bind has none yet, the source address itself; `source` is the wire address. The address formats are defined once, in [pb-points/src/addr.rs](../pb-points/src/addr.rs), and used by both the UI (when it lists binds) and the service (when it builds pack config). **Never build an address string by hand** — if the two sides disagree a bind silently never gets a value.

**JSON paths.** Every pack that digs a field out of JSON uses [pb-json](../pb-json/src/lib.rs) (`a.b`, `a[0].b`, `$.a`, `a.0` are all the same lookup).

**One pack per binary.** A crate that exports `pb_pack_register` cannot be linked into the same executable as another pack — the symbol clashes. Code two packs share must live in a plain `rlib` (`pb-tail` for file tailing, `pb-json`, `pb-points`). Likewise `pb-ui` must not depend on more than one pack crate (it depends on `pb-modbus` only for its codec types).

**State on disk.** Packs that resume (File follow, LAS) receive `state_dir` in their config — `<config dir>/file-follow` / `las-follow` — and save `{offset, inode}` per source there (`pb-tail`). A pack must work with `state_dir` empty (tests, read-only installs): it then starts at the configured position every run.

**Worker threads.** One thread per connection / file / target, each with an `Arc<AtomicBool>` stop flag and an `Arc<Mutex<Status>>` the main thread reads for `detail_json`. Use `pb_abi::worker::{sleep_stop, update_status, sanitize_id}`. Sleep in short slices (`sleep_stop`) so Stop is prompt.

## Ext packs: the short way to add a protocol

WITS, WebSocket, Sparkplug B, DNP3 and CAN are "ext" protocols: one generic path end to end.

1. **Address** — add an `ExtSpec` (section id, point prefix, leaf fields, required fields) to `EXT_PROTOCOLS` in `pb-points/src/addr.rs`.
2. **Engine slot** — add an `EngineId` variant (`pb-engine`), its title, pack id and note.
3. **Pack** — a crate with `pack.rs` holding `fn run(ctx: LinkCtx)` (the protocol), `endpoint` and `check`, then `pb_extkit::ext_pack_impl!` + `pb_abi::export_pack!`. `pb-extkit` owns workers, config diffing, output routing (`ctx.drain_writes()` / `ctx.latest`), quality helpers (`ctx.up / down / publish / publish_bad`) and the `seen` table the UI turns into one-click binds. Pure codecs go in `pb-proto` so tests and `pb-sim` can use them.
4. **UI** — add an `ExtProto` variant with a `UiSpec` (connection / Input / Output fields) in `pb-ui/src/ext_proto.rs`, plus `DockPane` / `HubProto` variants. Panel, hub inspector, persistence (`<id>` section of `project.json`), point allocation, Points and Mappings rows come for free.
5. **Service** — nothing: `build_ext_configure` turns hub ports (`ext_connection`, `ext_interval_secs`, `ext_inputs`, `ext_outputs`) into `{connections, inputs, outputs}` for every spec.
6. **Help** — an article in `help_catalog.rs`, and a device in `pb-sim` if the demos should use it.

## How config reaches a pack

The UI owns the editable project (connections in panels, binds on hub Input/Output nodes). The **service** turns that into each pack's configure JSON — `build_*_configure` in [pb-service/src/runtime.rs](../pb-service/src/runtime.rs) — so the pack never sees hub structure, only `{connections, inputs[], outputs[]}`. When you add a bind field you touch: the UI bind struct (`pb-ui/src/hubs.rs`), the `build_*_configure` function, the pack's config struct, and a unit test in `runtime.rs` that pins the JSON.

Rules the service applies that a pack should not repeat: a Modbus **server** connection only runs when a Modbus hub picks it (`hub.modbus_server`); an unset `enabled` means on; blank binds are dropped.

## Testing

* Unit tests live next to the code. Pack tests inject a fake host (`PbHostApi` with a capture callback) and drive `PackState` directly — see `pb-snmp/src/pack.rs` tests (fake agent) or `pb-files` (temp files via `pb_tail::testutil`).
* `scripts/test-all.sh` runs the workspace tests plus the end-to-end smokes (Modbus TCP, Modbus RTU over a pty pair, File follow, SNMP + LAS). Smokes start a real `pb-service` with a **private `XDG_CONFIG_HOME`**, because the UI finds the service through `~/.config/protocolbench/ipc.json` — a test service writing the shared file would hijack a running UI.
* The RTU test needs no hardware: `scripts/modbus-rtu-smoke.sh` links two pseudo-terminals.
* `scripts/field-sim.py` and `scripts/modbus-plant-sim.py` are long-running fake devices for manual demos (`../protocol-bench-projects/`).

## Known quirks and open items

These are deliberate or not-yet-fixed behaviours a reader would otherwise rediscover the hard way.

* **OPC UA `allow_anonymous` is not enforced.** The embedded server always offers anonymous login (user accounts are not implemented); the setting is stored but ignored (`pb-opcua/src/server.rs`).
* **Hub-broker MQTT outputs need a client.** An Output bound to the embedded broker's own connection id cannot publish (the broker has no publisher session). Bind the Output to a normal MQTT client — e.g. a loopback client on the broker's port, as `edge-logger.json` does.
* **`ipc.json` / `ipc.token` are not projects.** They are how the UI finds the service. The UI refuses to open or save a project over them, because the file dialog opens in that folder and overwriting `ipc.json` leaves the UI permanently "disconnected".
* **`project::load` / `project::save` in `pb-ui`** are legacy local-file helpers; the service owns `project.json` now and the UI goes through IPC. They are kept only until the remaining callers are removed.
* **Egress fan-out.** The engine sends point writes to every *connected* pack whose slot `takes_egress()`; a pack that connects later gets every current value once, then changes.
* **DNP3** is a TCP master only (no serial, no secure authentication, no outstation role). **Sparkplug B** handles scalar metrics (no DataSets / Templates) and does not publish a primary-host STATE. **CAN** needs Linux SocketCAN; the vcan test is skipped when `vcan0` is absent.
* **SNMP**: v1/v2c GET only (no v3, SET, walk or traps). **LAS**: `WRAP YES` files are refused; `STOP` is not rewritten while a log is being written; time-indexed logs use epoch seconds. **File follow**: CSV is one physical line per row. Windows file replacement is detected by shrink only (no inode there). New packs have only been run on Linux.

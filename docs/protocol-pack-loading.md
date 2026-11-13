# Protocol Bench — loading protocol packs into a free core

Date: 2026-11-13. Status: **settled — Option A (plugin ABI)**; **MQTT client pack landed first**. Complements [free-vs-paid-isolation.md](free-vs-paid-isolation.md) and PRD Settled #1 / #9 / #12 / #13.

## Verdict

**Settled:** the Protocol Bench **service** loads purchased protocol packs after download from a license server via a **stable C-style plugin ABI** into in-process `cdylib`s. Rust has no stable crate ABI — packs must cross that fixed C boundary into `pb-engine`. Sidecar IPC for packs (Option B) is deferred; all-in-one license-gated binary (Option C) is **rejected** for paid IP.

Entitlements are license-driven: **no fixed free protocol list** — the license defines which protocols/packs the customer gets. Offline grace ~**7 days** (phone-home to the license server; not a strict check on every operation).

## MQTT client pack (first implementation)

**Client-first.** `pb-mqtt` is a `cdylib` (+ `rlib` for unit tests) that speaks MQTT via **rumqttc 0.24** (`websocket` + `use-rustls`). It is a **source / output** client — **not** an embedded broker. Hub MQTT broker role is **deferred**.

| Piece | Role |
| --- | --- |
| `pb-abi` | C ABI: `pb_pack_abi_version`, `pb_pack_register`, opaque handles, host callbacks (point upsert + log); host refuses load on ABI skew |
| `pb-points` | Host-owned typed point store; packs upsert only through ABI callbacks |
| `pb-engine` | `libloading` modules-dir loader, verify stub (signature later), wrap pack as `ProtocolEngine`; commands: configure / connect / disconnect / subscribe / publish / write_point |
| `pb-mqtt` | Connect / disconnect / reconnect; last error + quality; subscribe → sequential short point ids (`mqtt_1`, `mqtt_2`, …; never reused) with source metadata `mqtt/<client_name>/<topic>` (JSON fields `…::path`); point write / publish → broker; TLS / WSS; password in JSON config (no keyring) |
| `pb-service` | Headless binary: load `modules/`, run host |
| UI MQTT panel | Live snapshot (connected, topics, last error) + fields to connect one client |

### Load the pack

```bash
# From the repo root:
cargo build -p pb-mqtt
mkdir -p modules
cp target/debug/libpb_mqtt.so modules/   # Windows: pb_mqtt.dll → modules/

# Optional smoke (requires Mosquitto or any broker on 1883):
./scripts/mqtt-smoke.sh

# UI (loads ./modules or $PB_MODULES):
PB_MODULES="$PWD/modules" DISPLAY=:0 cargo run -p pb-ui

# Headless service:
PB_MODULES="$PWD/modules" cargo run -p pb-service
```

If Mosquitto is not installed, start any broker on `127.0.0.1:1883`, then use the MQTT panel for connections and hub **Input** / **Output** nodes for point binds (or the smoke script).

### MQTT runtime model

1. **MQTT panel — connections only**  
   Broker, client id (optional Unique / random on connect), credentials, transport, Apply/Connect, Disconnect. Optional **wide subscribe** filters (`pb/#`, `sensor/+/temp`, `#`, …) for traffic / bring-up. Wildcards (`+`, `#`) are supported end-to-end in pack subscribe + sync.

2. **Hub Input / Output nodes — point binds**  
   - **Input:** pick a connection (many Inputs may share one broker) + topic/field → creates points. Point ids are always auto sequential `mqtt_N` (never reused in a project; no manual picker).  
   - **Output:** pick connection + publish topic + points.  
   Overlap of the same topic/field across Inputs is OK.

3. **Topic tree**  
   TopicBench-style hierarchy from subscribed traffic. Shown in the MQTT panel (preview) and the hub Input inspector (click leaf → bind). Fed from pack `clients[].traffic` in `topics_json`, **merged with live point sources** (`mqtt/<client>/<topic>`) so the tree matches the Points panel even if the traffic snapshot is empty or mid-truncate. Host `read_cstr` grows the buffer (was 512) so `#` floods do not clear the tree.

4. **Points**  
   Only hub Input–defined paths become host points. Source metadata: `mqtt/<client>/<topic>[::path]`. Persist under `points` + `next_point_seq`.

### Project JSON (MQTT connections + hub binds)

Editable wire config (not dock layout). Default path: `…/ProtocolBench/project.json` (via `directories` — e.g. `~/.config/protocolbench/project.json`). File → Open / Save as project uses a path dialog (non-blocking). MQTT panel and hub Input/Output bind changes autosave here.

- **`mqtt.clients[]`** — connections + wide subscribe only (no Inputs/Outputs editors on the panel).  
- **`hubs`** — diagram + per-port `mqtt_connection` / `mqtt_inputs` / `mqtt_outputs` (v4+). Older files without `hubs` keep the diagram from `ui.toml` until the next save.  
- **`points` / `next_point_seq`** — short id bindings.

```json
{
  "version": 4,
  "points": [
    { "id": "mqtt_1", "source": "mqtt/local/pb/in" },
    { "id": "mqtt_2", "source": "mqtt/local/pb/in::temp" }
  ],
  "next_point_seq": 3,
  "mqtt": {
    "selected_id": "local",
    "clients": [
      {
        "id": "local",
        "label": "Local broker",
        "host": "127.0.0.1",
        "port": 1883,
        "client_id": "pb-local",
        "unique_client_id": false,
        "username": "",
        "password": "",
        "transport": "tcp",
        "subscriptions": ["pb/#"],
        "auto_connect": true
      }
    ]
  },
  "hubs": {
    "hubs": [
      {
        "id": 6,
        "name": "Edge",
        "protocol": "Mqtt",
        "sources": [
          {
            "id": 7,
            "protocol": "Mqtt",
            "label": "Local broker",
            "mqtt_connection": "local",
            "mqtt_inputs": [
              { "topic": "pb/in", "path": "", "point": "" },
              { "topic": "pb/in", "path": "temp", "point": "" }
            ]
          }
        ],
        "outputs": [
          {
            "id": 8,
            "protocol": "Mqtt",
            "label": "Upstream",
            "mqtt_connection": "local",
            "mqtt_outputs": [
              { "point": "mqtt_1", "topic": "out/x", "qos": 0, "retain": false }
            ]
          }
        ]
      }
    ]
  }
}
```

Legacy client-level `parse_rules` / `outputs` on MQTT clients migrate onto hub Input/Output nodes on load. `ui.toml` remains layout/theme (and a hub fallback until project v4 hubs are saved). Password is stored in the project JSON for now (no keyring).

## Option A — in-process `cdylib` + fixed C ABI — **settled**

Pack ships as `libpb_mqtt.so` / `pb_mqtt.dll` (and later `pb_modbus.so`, etc.). Core uses `libloading`, calls a versioned entry (`pb_pack_abi_version`, `pb_pack_register`), and gets opaque handles that the host wraps as `pb-engine` trait objects.

**Pros**

- One service process; lowest latency; same thread model as today’s engine slots.
- Matches “driver pack” mental model; Windows MSI (`cargo-wix` / WiX) drops files into the module dir beside service + UI.
- Paid code stays off the core disk until purchase / entitlement download.

**Cons**

- **ABI pain:** versioned C structs, careful ownership/lifetime rules, no `String` / `Vec` across the boundary without explicit alloc/free or host-owned buffers.
- Service and pack major ABI must match; skew = load refusal, not silent UB.
- A pack crash can take down the whole service process.
- Needs **signed packs** + verify-before-load (same key story as licenses, or a dedicated pack-signing key).

This is the product decision for packs, licensing, and protocol source isolation.

## Option B — sidecar process + localhost IPC — **deferred**

Pack is its own binary; core spawns it (or connects if already running) and talks over a local socket / pipe. Engine trait boundary stays in core; the sidecar implements the protocol wire.

**Pros**

- Stronger crash isolation; pack language can differ later.
- Clearer process-level sandboxing story for plant PCs.

**Cons**

- Heavier UX (lifecycle, restart, multi-pack port/pipe naming, Windows service vs user session).
- Higher latency and more serialization work for every point update.
- More packaging surface (N binaries + core).

Deferred until crash isolation or third-party pack authors become real requirements. Not the v1 path. (Separate from UI↔service local IPC, which is follow-on design under PRD Settled #1.)

## Option C — license-gated code in one binary — **rejected for paid IP**

All engines compile into the free install; a signed entitlement list unlocks paid ones. Simplest ship path, weakest anti-RE: paid protocol code sits on every free disk. **Rejected** as the paid-IP path for Protocol Bench (paid source must stay in packs). Keep only as historical contrast.

## Installer + license server flow

1. User installs **core** on Windows via **MSI (WiX / `cargo-wix`)** — **service + UI + module dir** (Linux installer later / TBD). Upgrades must **preserve config, licenses, and packs**.
2. App **login / activate** against the central license server (PC transfer, enterprise accounts later).
3. Server returns **entitlements** (pack IDs + expiry / seats) — the license defines which protocols/packs the customer gets (**no fixed free list**).
4. App **downloads signed pack artifacts** for those entitlements (same license-server path as product updates: **manifest + signed downloads** — installer choice does not replace updates).
5. Service **verifies signature** (and ABI version) before accepting the file.
6. Packs land in a known **module directory** (`…/protocol-bench/modules/` or beside the install).
7. On start (or after download), the **service** **registers engines** from verified packs into the `pb-engine` registry.

Offline after purchase: verify local signature + cached entitlement; phone-home for renewals / transfer with ~**7-day** offline grace (not a strict check on every operation). Windows installer settled (PRD Settled #13); auto-updater poll / UX and Linux install channel still TBD (PRD Still open #9).

## What stays in core vs pack

| In core (service + UI) | In pack |
| --- | --- |
| Points model, hub graph, mappings, project I/O (service) | Protocol wire + device session |
| Engine registry, command/snapshot boundary (service) | Pack-specific config schema + defaults |
| egui UI, dock layout, **panel stubs / locked placeholders** for unavailable slots | **Panel hooks** (or pack-owned panel draw callbacks) once loaded / entitled |
| License client, pack download + verify | Pack signing metadata / manifest |
| Engines allowed by the customer’s **entitlement** (license-defined; no fixed free list) | Engines delivered as packs for entitled protocols |

Hub canvas and points store never live in the pack. Packs speak into the service point bus through the engine trait / ABI. The UI does not load packs.

## Relation to PRD Settled #1 (service + UI)

**Settled:** service always + separate UI. Packs load in the **service**; the UI only needs panel hooks or remote snapshots over local IPC (IPC design is follow-on). Shared module dir and loader live with the service.

## Settled recommendation

1. Keep the **`pb-engine` trait boundary** as the internal contract (in the service).
2. **Option A settled:** in-process `cdylib` packs + **signed downloads** from the license server.
3. **Entitlement-driven** packs — no fixed free protocol list; license defines what the customer gets.
4. **Defer Option B** (pack sidecars) unless pack crashes or hostile third-party packs force process isolation.
5. **Do not** implement Option C as the paid-IP path.

MQTT client pack + ABI/loader/service scaffold are implemented in-tree. License-server download, pack signing enforcement, and MSI remain follow-on. Windows MSI via WiX / `cargo-wix` is settled (PRD Settled #13). Remaining open: auto-updater poll / UX, Linux installer (later / TBD), embedded MQTT broker for hubs, and further license-server product details. Offline grace (~7 days) is settled.

## Open questions

1. **ABI versioning** — semver on the C ABI, and how many service majors a pack must support?
2. ~~**Free pack list**~~ — **settled:** no fixed free list; entitlement-driven (PRD Settled #9).
3. ~~**Offline grace**~~ — **settled:** ~7 days (phone-home; not every check).
4. **Windows DLL load rules** — SafeDllSearchMode, install-dir vs `%LOCALAPPDATA%` modules, and whether packs need Authenticode in addition to app-level ed25519 signatures?
5. ~~**Windows installer**~~ — **settled:** MSI via WiX / `cargo-wix` (not Inno Setup); service + UI + module dir; preserve config / licenses / packs on upgrade (PRD Settled #13).
6. **Auto-updater / Linux install** — update poll / UX and Linux channel still TBD (PRD Still open #9); updates remain license-server manifest + signed downloads.

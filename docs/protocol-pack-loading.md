# Protocol Bench — loading protocol packs into a free core

Date: 2026-11-13. Status: **settled — Option A (plugin ABI)**. Complements [free-vs-paid-isolation.md](free-vs-paid-isolation.md) and PRD Settled #9 / Still open #9 / #12.

## Verdict

**Settled:** a free Protocol Bench core loads purchased protocol packs after download from a license server via a **stable C-style plugin ABI** into in-process `cdylib`s. Rust has no stable crate ABI — packs must cross that fixed C boundary into `pb-engine`. Sidecar IPC (Option B) is deferred; all-in-one license-gated binary (Option C) is **rejected** for paid IP.

## Option A — in-process `cdylib` + fixed C ABI — **settled**

Pack ships as `pb_modbus.so` / `pb_modbus.dll` (etc.). Core uses `libloading`, calls a versioned entry (`pb_pack_abi_version`, `pb_pack_register`), and gets opaque handles that the host wraps as `pb-engine` trait objects.

**Pros**

- One process; lowest latency; same thread model as today’s engine slots.
- Matches “driver pack” mental model; installer can drop files into a module dir.
- Paid code stays off the free disk until purchase.

**Cons**

- **ABI pain:** versioned C structs, careful ownership/lifetime rules, no `String` / `Vec` across the boundary without explicit alloc/free or host-owned buffers.
- Shell and pack major ABI must match; skew = load refusal, not silent UB.
- A pack crash can take down the whole process.
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

Deferred until crash isolation or third-party pack authors become real requirements. Not the v1 path.

## Option C — license-gated code in one binary — **rejected for paid IP**

All engines compile into the free install; a signed entitlement list unlocks paid ones. Simplest ship path, weakest anti-RE: paid protocol code sits on every free disk. **Rejected** as the paid-IP path for Protocol Bench (paid source must stay in packs). Keep only as historical contrast.

## Installer + license server flow

1. User installs **free core** (installer / bare binary) with free engines only.
2. App **login / activate** against the central license server (PC transfer, enterprise accounts later).
3. Server returns **entitlements** (pack IDs + expiry / seats).
4. App **downloads signed pack artifacts** for those entitlements.
5. Core **verifies signature** (and ABI version) before accepting the file.
6. Packs land in a known **module directory** (`…/protocol-bench/modules/` or beside the install).
7. On start (or after download), core **registers engines** from verified packs into the same `pb-engine` registry free engines use.

Offline after purchase: verify local signature + cached entitlement; phone-home only for renewals / transfer (grace policy TBD — still open under PRD #9 details).

## What stays in core vs pack

| In free core | In pack |
| --- | --- |
| Points model, hub graph, mappings, project I/O | Protocol wire + device session |
| Engine registry, command/snapshot boundary | Pack-specific config schema + defaults |
| egui shell, dock layout, **panel stubs / locked placeholders** for known paid slots | **Panel hooks** (or pack-owned panel draw callbacks) once loaded |
| License client, pack download + verify | Pack signing metadata / manifest |
| Free engines (a couple of simple protocols — list TBD #12) | Paid engines (Modbus, MQTT, OPC UA, oilfield, … — TBD #12) |

Hub canvas and points store never live in the pack. Packs speak into the core point bus through the engine trait / ABI.

## Relation to PRD still-open #1 (one binary vs service + UI)

Packs work either way. A single GUI binary and a headless service can share the same module dir and loader. If the product later splits “service + bench UI,” the **service** loads packs; the UI only needs panel hooks or remote snapshots. Do not block pack design on #1.

## Settled recommendation

1. Keep the **`pb-engine` trait boundary** as the internal contract.
2. **Option A settled:** in-process `cdylib` packs + **signed downloads** from the license server.
3. Ship free engines compiled into core; paid engines only as packs.
4. **Defer Option B** unless pack crashes or hostile third-party packs force process isolation.
5. **Do not** implement Option C as the paid-IP path.

No packaging or license-server code in this pass — design only. Remaining open items are free protocol list, license-server / installer / updater details, and offline grace.

## Open questions

1. **ABI versioning** — semver on the C ABI, and how many shell majors a pack must support?
2. **Free pack list** — which engines ship in core vs first paid packs (PRD #12)?
3. **Offline grace** — how long after entitlement expiry / without phone-home do packs keep running?
4. **Windows DLL load rules** — SafeDllSearchMode, install-dir vs `%LOCALAPPDATA%` modules, and whether packs need Authenticode in addition to app-level ed25519 signatures?

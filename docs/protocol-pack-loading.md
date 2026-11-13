# Protocol Bench — loading protocol packs into a free core

Date: 2026-11-13. Status: **settled — Option A (plugin ABI)**. Complements [free-vs-paid-isolation.md](free-vs-paid-isolation.md) and PRD Settled #1 / #9 / #12 / #13.

## Verdict

**Settled:** the Protocol Bench **service** loads purchased protocol packs after download from a license server via a **stable C-style plugin ABI** into in-process `cdylib`s. Rust has no stable crate ABI — packs must cross that fixed C boundary into `pb-engine`. Sidecar IPC for packs (Option B) is deferred; all-in-one license-gated binary (Option C) is **rejected** for paid IP.

Entitlements are license-driven: **no fixed free protocol list** — the license defines which protocols/packs the customer gets. Offline grace ~**7 days** (phone-home to the license server; not a strict check on every operation).

## Option A — in-process `cdylib` + fixed C ABI — **settled**

Pack ships as `pb_modbus.so` / `pb_modbus.dll` (etc.). Core uses `libloading`, calls a versioned entry (`pb_pack_abi_version`, `pb_pack_register`), and gets opaque handles that the host wraps as `pb-engine` trait objects.

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

No packaging or license-server code in this pass — design only. Windows MSI via WiX / `cargo-wix` is settled (PRD Settled #13). Remaining open: auto-updater poll / UX, Linux installer (later / TBD), and further license-server product details. Offline grace (~7 days) is settled.

## Open questions

1. **ABI versioning** — semver on the C ABI, and how many service majors a pack must support?
2. ~~**Free pack list**~~ — **settled:** no fixed free list; entitlement-driven (PRD Settled #9).
3. ~~**Offline grace**~~ — **settled:** ~7 days (phone-home; not every check).
4. **Windows DLL load rules** — SafeDllSearchMode, install-dir vs `%LOCALAPPDATA%` modules, and whether packs need Authenticode in addition to app-level ed25519 signatures?
5. ~~**Windows installer**~~ — **settled:** MSI via WiX / `cargo-wix` (not Inno Setup); service + UI + module dir; preserve config / licenses / packs on upgrade (PRD Settled #13).
6. **Auto-updater / Linux install** — update poll / UX and Linux channel still TBD (PRD Still open #9); updates remain license-server manifest + signed downloads.

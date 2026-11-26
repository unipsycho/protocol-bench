# PRD gap review

Reviewed 2026-11-19, updated 2026-11-26 against the working tree. `[x]` done, `[ ]` open.

## 2026-11-26 pass — restart behaviour, project loading tests, UI fit

- [x] **Reopen the last project after a restart.** Cause: the service always started on the default `project.json`, and the HTTP pack reported "connected" as soon as any client was configured (every sample has one), so the UI saw "live hubs", attached to the default project and never opened yours. Fixes: the service writes `<config dir>/last-project` on open / save-as and starts from it (`pb_ipc::startup_project_path`); HTTP reports connected only after a request succeeds; a UI that starts before the service opens the last project once the service appears. Tests: `pb-service/tests/restart_reopens_project.rs` runs the real binary, kills it with SIGKILL and checks the same project comes back (fails against the old start-up code).
- [x] **Malformed project files** are refused without moving the service's project path (a later save used to land on top of the broken file). **IPC client** keeps the connection on an error reply instead of dropping it. Tests: `pb-service/tests/ipc_contract.rs` (auth, hello-first, open / get / put, new-project path, malformed files, idle status).
- [x] **No placeholder connections.** Opening a project with an empty MQTT / OPC UA / HTTP / Streams / LAS / SNMP / Modbus / File follow list added a stub (an enabled Modbus client on 127.0.0.1:502, a `/dev/ttyUSB0` serial port, …) and autosave wrote it back. Lists now stay empty. Tests: `pb-ui/src/project_load_tests.rs` (every section file → panels → file for the demos, the sample folder, a sparse project; old / unknown / malformed files; layout seeding; point bindings), checked with a mutant.
- [x] **File → New project…** writes an empty project (`ProjectFile::blank`) and opens it.
- [x] UI fit in narrow panes: Hubs toolbar hint shortens instead of widening the pane (which made the register map overflow), Points table drops Source before cropping names, Mappings keeps To over From and hides Hub for single-hub projects, protocol picker no longer covers Pop out, connection rows elide under the ×, form labels narrow, MQTT Unique checkbox fits, Traffic listener address wraps, Help "Read ›" no longer overlaps. Status bar says "13 protocol packs · 69 points · 3 live links" instead of a debug dump. Settings lists the loaded packs and folders (the old "Five slots" text is gone).
- [x] Test hygiene: IPC frame tests (round trip, size cap, truncation, one write per frame), HTTP pack link tests with a local server, OPC UA server tests on free ports with a temp PKI (one used the real `~/.local/share` store), MCP test waits for the first snapshot (was flaky). `PB_NO_TRAY=1` for headless service runs.

### Still open after this pass

- [ ] Saved projects that went through the old UI keep their placeholder connections (`com1`, `meter`, `api`, `plant`, `switch` in most samples) until deleted by hand.
- [ ] Protocol picker list scrolls after ~10 entries (egui popup height); every protocol is still reachable.

## 2026-11-25 pass — register maps, five new protocols, MCP, demo plants

- [x] **Modbus register map editor** (`pb-ui/src/register_map.rs`) for hub Inputs, Outputs and Server maps: + Register / + Block, device templates (power meter, VFD status / command, tank, frac pump, flow computer, pump commands), CSV import / export (foreign headers, Modicon refs), Modicon / 0-based address toggle, memory strip per area (overlaps red on servers), table with tag name / area badge / type / scaling / point / live value, inline edit card, point picker, request-count summary. 8 tests.
- [x] **Engineering scaling + tag names + units** on Modbus binds (`scale`, `offset`, `name`, `units`); pack applies scaling on reads and inverts it on client writes and server-map pushes (`codec::Scaling`, tests incl. an end-to-end pack test). f32 values widen to their shortest decimal (`21.7`, not `21.700000762939453`).
- [x] **Ext protocol framework**: `pb_points::addr::EXT_PROTOCOLS` (shared address rules), `EngineId` slots, generic `build_ext_configure` + point materialisation in the service, `pb-extkit` pack runtime, schema-driven UI (`pb-ui/src/ext_proto.rs`: panel, hub inspector with "seen on the wire" chips, persistence).
- [x] **WITS** (`pb-wits`): level 0 blocks over TCP client / listen / serial, both directions, null values and stale detection → bad quality. 7 tests.
- [x] **WebSocket** (`pb-websocket`, tungstenite): client (ws / wss, subscribe message) and server (fans out to browsers, current picture on connect, reads their messages). 3 tests.
- [x] **Sparkplug B** (`pb-sparkplug` + `pb-proto::sparkplug` hand-written protobuf): host (BIRTH aliases, DATA, DEATH → bad, NCMD / DCMD) and edge node (NBIRTH / DBIRTH / NDATA / DDATA, bdSeq, NDEATH will, rebirth). End-to-end test through an embedded broker.
- [x] **DNP3** (`pb-dnp3` + `pb-proto::dnp3`): TCP master — link CRC, transport reassembly, class 0 / 1 / 2 / 3 polls, g1/g2/g10/g11/g20/g22/g30/g32/g40/g42, ONLINE flag → quality, restart clear, CROB + analog output controls; demo outstation. 6 tests.
- [x] **CAN** (`pb-can` + `pb-proto::can`): SocketCAN raw sockets, DBC-style Intel / Motorola signals, signed / float, J1939 `pgn:` matching, frame transmit. Codec tests; live vcan test runs when `vcan0` exists.
- [x] **Engine egress** goes to every connected pack that takes writes; a pack that connects late gets every current value once.
- [x] **MCP server on the service** (`pb-service/src/mcp.rs`, PRD #14): Streamable HTTP on 127.0.0.1, bearer auth (IPC token), 11 tools. Tests + live HTTP check.
- [x] **Demo plants** (`pb-sim`, all Rust): frac spread, bottling line, lift station projects in `../protocol-bench-projects/`; `scripts/demo-smoke.sh` runs each through the real service (all three pass). `PB_AUTO_RUN_HUBS=1` for headless runs; `PB_UI_SELECT=hub:node` opens a node at UI start.
- [x] UI: header hub state follows the service even when the MQTT / OPC UA panels are closed; help articles for every new protocol, the register map editor, the demos and MCP.

### Still open after this pass

- [ ] Modbus Bench pass and real hardware for every protocol (all new packs are tested against in-repo fakes / simulators only; CAN only if `vcan0` exists).
- [ ] DNP3 over serial, DNP3 outstation role, secure authentication. Sparkplug DataSets / Templates / primary-host STATE. SNMPv3 / SET / traps. WITSML.
- [ ] MCP edits are not pushed to an open UI (it keeps its own copy until the project is reopened).
- [ ] Windows: new packs untested there (CAN reports "needs Linux"); MSI not started.
- [ ] Two-serial + two-TCP acceptance test has no automated coverage; restart now restores the same project (tested), bridge traffic after restart is not asserted.

## Done in the 2026-11-19 pass (UI)

- [x] **Modbus panel** — connections: client / server, TCP / RTU, serial line settings, unit, timeout, retries (`pb-ui/src/modbus_panel.rs`).
- [x] **Modbus hub node configs** — Input (client, poll, register binds → `modbus_N`), Output (point → coil / holding), hub Server picker.
- [x] **File follow panel** — sources: path, CSV / JSON lines, delimiter, header row, poll, start position, rotation (`files_panel.rs`); hub **Files Input** binds column / path → `file_N`.
- [x] **Mappings panel** — read-only route list (point, from, to, hub) derived from hub Outputs (`mappings_panel.rs`).
- [x] Project file v9: `modbus`, `files` sections (older files load with defaults); Points panel, point-id allocation and dirty-save wired for both.
- [x] Default dock layout contained only 10 of 13 panes (two tests failed on `main`) — HTTP / LAS / SNMP now share a tab strip with File follow.
- [x] PRD refreshed to match the code (live packs, panels, IPC, project version, paths); added root `README.md`; `pb-http` added to the service README / smoke script build lines.

## Open — needs a pack or a decision

- [ ] Modbus is done in the pack for TCP + RTU client and server, but only exercised against fakes: no real device, no real serial port, and no Modbus Bench pass yet (phase 7).
- [x] Live link state, quality and last error per endpoint in the UI (Modbus phase 6, then every pack).
- [x] MCP server on the service (PRD #14) — built 2026-11-25.
- [x] Sparkplug B, DNP3, WebSocket, CAN, WITS — built 2026-11-25 (see above).
- [ ] Windows: service host is scaffolded only; MSI / `cargo-wix` not started (PRD says hold until license details land).
- [x] Output binds exist for MQTT, Modbus, Streams, HTTP, LAS, WITS, WebSocket, Sparkplug B, DNP3 and CAN. Still input-only: OPC UA client writes, File follow, SNMP (no SET).
- [ ] Two-serial + two-TCP acceptance test (PRD "Done when") has no automated coverage.

## SNMP, LAS, RTU test, column picker — done

- [x] **`pb-snmp`**: own BER / SNMP v1 + v2c GET codec (known-byte tests), UDP client with batching / tooBig / noSuchName handling / retries, per-target workers, bad quality + recovery. 16 tests + end-to-end via `scripts/snmp-las-smoke.sh` (fake agent, going silent and coming back).
- [x] **`pb-las`**: LAS 1.2 / 2.0 header + row parser, follow (growing file, NULL → bad, offset resume, once mode, curve list), write (depth- or time-indexed rows, header creation, refuses mismatching existing files). 14 tests + smoke.
- [x] Engine slots `Snmp` / `Las` (pack ids `snmp` / `las`), service configure + point ids, LAS egress writes; UI: live status on both panels and hub inspectors, live values on bind rows.
- [x] Shared **`pb-tail`** crate (tailing + saved state) so File follow and LAS do not duplicate it (and do not collide on pack entry points).
- [x] **Column picker**: File follow columns / JSON paths with sample values and LAS curves as clickable chips, via the pack (preview mode when there are no binds). Verified through the real service in `scripts/files-smoke.sh`.
- [x] `scripts/test-all.sh` runs the workspace tests and every smoke (`--quick` = unit tests only).
- Known limits: SNMPv3 / SET / walks / traps; LAS `WRAP YES`, `STOP` not rewritten while writing, time index is epoch seconds; the Windows build of the new packs is untested.

## File follow — done

- [x] `pb-files` pack: complete-line tailing, saved byte offset + file identity, catch-up after restart without re-reading, truncate / replace (rotation) → restart with a new header, missing file → bad quality + recovery, typed cells, CSV (quoted fields, name or index columns) and JSON-lines (dotted paths). 22 pack tests + `scripts/files-smoke.sh` end to end through `pb-service`.
- [x] Service wiring (`build_files_configure`, `file_N` ids, offset dir in the config dir) and UI live state (Files panel status / rows / offset, hub Files Input status + live values).
- Known limits: one physical line = one CSV row (no multi-line quoted fields); the path is read where the service runs; on Windows file replacement is detected by shrink only (no inode); no column picker / header preview in the UI yet.

## Modbus build plan — progress

- [x] Decisions: per-bind data types + byte / word order; hub picks one Server connection; server register map lives on the Server connection, separate from client binds.
- [x] **Phase 2** — `pb-modbus` codec (all types / orders, bit + string, read-block planner), 12 tests.
- [x] **Phase 1** — model + UI: extended `ModbusBind` (`order`, `bitN`, `strN`, u64 / i64 / f64), shared register editor, validation, server register map editor, project v10.
- [x] **Phase 3** — `pb-modbus` client pack: own MBAP / RTU framing (no tokio), batched polling, quality bad + backoff reconnect, retries, typed writes (incl. bit read-modify-write); 29 pack tests + `scripts/modbus-smoke.sh` end to end through `pb-service`.
- [x] **Phase 4** — `pb-modbus` server: TCP listener + RTU slave serving each Server connection's register map; host points mirror into the image, master writes mirror back to host points; bind failure retried; only runs when a Modbus hub picks it. 40 pack tests; `scripts/modbus-smoke.sh` bridges a device value through the server to a master and back.
- [x] **Phase 5** — service wiring: `build_modbus_configure`, `modbus_N` point ids, Output writes routed to the pack, build scripts.
- [x] **Phase 6** — live state in the UI: per-connection status dot / state / last error / poll + master counts in the Modbus panel, the hub Input / Output / hub-server inspectors, live value + quality dots on bind rows and server-map rows, an **Enabled** switch per connection, and Modbus listeners / connected clients in the Traffic panel. (Traffic stays stats-only per PB-107 — no frame trace.)
- [x] **Phase 7** — loopback + smoke (`scripts/modbus-smoke.sh`, in-process loopback tests, Plant demo run) and **RTU over a virtual serial pair** (`scripts/modbus-rtu-smoke.sh`: real serial driver on a pty pair, both parities, 125-register frames, exceptions, silent wrong unit, broadcast, 200-round soak). Still open: a Modbus Bench pass and real hardware.
- [x] **Phase 8** — realistic Plant sample: Tank PLC (TCP; f32 CDAB level / temp, u32 flow, coil, discrete, alarm bit), Power meter (RTU, unit 3, u16 + f32 BADC), hub Modbus server serving those points, MQTT publishes, and an MQTT `plant/pump/cmd` → Modbus coil Output. `scripts/modbus-plant-sim.py` simulates the PLC; `../protocol-bench-projects/modbus-plant.json` is the runnable demo.

## Questions for you

Answered 2026-11-19: Modbus word order is per bind (not assumed); one Server connection per hub; server map is separate from client binds; Mappings stays read-only for now; missing research docs will come from your other agent; Plant sample gets realistic Modbus once the pack works (phase 8).

Client register binds stay on hub Input / Output nodes — confirmed: those nodes represent devices.

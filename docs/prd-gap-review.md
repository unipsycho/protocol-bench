# PRD gap review

Reviewed 2026-11-19 against the code on `main`. `[x]` done, `[ ]` open.

## Done in this pass (UI)

- [x] **Modbus panel** — connections: client / server, TCP / RTU, serial line settings, unit, timeout, retries (`pb-ui/src/modbus_panel.rs`).
- [x] **Modbus hub node configs** — Input (client, poll, register binds → `modbus_N`), Output (point → coil / holding), hub Server picker.
- [x] **File follow panel** — sources: path, CSV / JSON lines, delimiter, header row, poll, start position, rotation (`files_panel.rs`); hub **Files Input** binds column / path → `file_N`.
- [x] **Mappings panel** — read-only route list (point, from, to, hub) derived from hub Outputs (`mappings_panel.rs`).
- [x] Project file v9: `modbus`, `files` sections (older files load with defaults); Points panel, point-id allocation and dirty-save wired for both.
- [x] Default dock layout contained only 10 of 13 panes (two tests failed on `main`) — HTTP / LAS / SNMP now share a tab strip with File follow.
- [x] PRD refreshed to match the code (live packs, panels, IPC, project version, paths); added root `README.md`; `pb-http` added to the service README / smoke script build lines.

## Open — needs a pack or a decision

- [ ] Modbus is done in the pack for TCP + RTU client and server, but only exercised against fakes: no real device, no real serial port, and no Modbus Bench pass yet (phase 7).
- [ ] Live link state, quality and last error per endpoint in the UI — the Modbus pack reports state / error / client count per connection already (`topics_json`); the panel does not show it yet (phase 6). 
- [ ] MCP server on the service (PRD #14 — no schema yet).
- [ ] Sparkplug B, DNP3, WebSocket, CAN, WITS — in the PRD protocol list, nothing built.
- [ ] Windows: service host is scaffolded only; MSI / `cargo-wix` not started (PRD says hold until license details land).
- [ ] Output binds exist for Modbus (client writes) only; OPC UA, File and SNMP outputs are not available.
- [ ] Two-serial + two-TCP and restart-restores-bridges acceptance tests (PRD "Done when") have no automated coverage.

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

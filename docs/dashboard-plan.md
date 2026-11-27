# Dashboard output pack — plan

A new **Dashboard** output: the service hosts a web page where the user builds grid dashboards (gauges, numerics, strings, multi-point charts) wired to points. It is the "viewing site" for everything Protocol Bench converts. Builds on the WebSocket server's built-in viewer page, which stays as the raw-feed debug view.

## What the user gets

- Hub **Output** node → Dashboard connection; the bound points are what the dashboard can show.
- Browser opens `http://host:port/` (button in the Dashboard panel, same as the WebSocket viewer button).
- **Tiles** on a snapping grid, free placement with gaps allowed: drag a tile by its grip to move it, drag any corner or edge to resize, drop it on another tile to swap places or split that tile's space.
- Each tile has a small **edit icon**: flips the tile to edit mode (pick point, widget type, title, units, decimals, min/max, colours, thresholds), flip back to view mode. A page-level Edit toggle shows or hides all tile chrome.
- **Multiple named dashboards**: top button bar / dropdown to switch; add, rename, duplicate, delete, set the default. Deep link `/#/<name>`, plus `?kiosk` for chrome-less display.
- Fonts scale with the tile (auto-fit), so a numeric can be huge on a wall screen.

## Widgets (v1)

| Widget | Shows |
| --- | --- |
| Numeric | big value + units, decimals, auto-fit font |
| Text / string | string points, or a static label / heading |
| Gauge | radial arc, min / max, coloured zones (warn / alarm) |
| Bar / level | linear fill, same zones |
| Indicator | boolean LED with on / off text and colours |
| Chart | time-series chart with **several points on one chart** (see below) |

Every widget dims and shows a badge when the point's quality is BAD or the value is stale, and shows "—" before the first value. (Matches the pack rule: hold the value, flag the quality.)

## Architecture

```
hub Output ──► pb-dashboard pack (cdylib, in the service)
                 ├─ HTTP  GET /            → web app (embedded in the .so)
                 ├─ HTTP  /api/dashboards  → list / load / save / delete layouts (JSON)
                 └─ WS    /ws              → points snapshot on connect, then changes
browser: wasm web app ◄──────────────────────────┘
```

- **Pack `pb-dashboard`**: an ext pack on `pb-extkit` like WITS / WebSocket (`EngineId::Dashboard`, `ExtProto::Dashboard`, `dashboard` project section, `build_ext_configure`). Connection fields: `host`, `port`, `title`, `allow_edit`, `default_dashboard`, optional `password`.
- **Shared server code**: the HTTP-or-WS accept loop, peek-for-upgrade and static serving now in `pb-websocket/src/pack.rs` move to a plain rlib (`pb-webserve`), because two packs can't link into one binary. `pb-websocket` and `pb-dashboard` both use it. Needs a small HTTP/1.1 router (GET / PUT / DELETE, content-length bodies, 404 / 405) — hand-rolled on `TcpListener` like today, no new async runtime.
- **Wire format** (server → browser): `{"t":"snap","points":{"psi":{"v":3400,"q":0,"u":"psi","ts":…}}}` on connect, then `{"t":"chg","points":{…}}` batched at the Output interval. Bind metadata (units, min / max, label) comes from the Output bind and seeds new tiles' defaults. The browser only ever sees points bound to that connection.
- **Layout storage**: server-side JSON files, one per dashboard, so layouts survive browsers and are shared by everyone viewing. Written by the pack via the REST API (atomic temp-file + rename). Location: `<config dir>/dashboards/<connection id>/<name>.json`, the dir passed to the pack in its config (the pack has no `pb_ipc` access). Layout JSON = tile tree + per-tile widget config, versioned (`"v":1`).
- **Write-back**: see the dedicated section below.

## Web front end: Rust → WASM (egui)

Everything stays Rust (project rule). The web app is an **eframe web build** (`wasm32-unknown-unknown`): the Rust code is compiled to WebAssembly, a compact binary format every browser runs natively, and drawn on a canvas. The pack just serves three static files (`index.html`, a small generated JS loader, the `.wasm`); nobody writes JS by hand, and the browser needs nothing installed.

- **Own tiling, nothing shared with `pb-ui`.** The dashboard gets its own small tiling module inside `pb-dash-web`, simpler and more widget-specific than the desktop dock (design below). It borrows ideas from the main app but no code is extracted, so `pb-ui` is untouched and there is no refactor risk.
- **Crate `pb-dash-web`**: the wasm app — tiling, widgets, edit panel, dashboard switcher, WS client (`web-sys` WebSocket), REST via `fetch`. Shared types (layout JSON, wire messages) in a small `pb-dash-model` rlib used by both the pack and the wasm app, so the schema can't drift.

### Dashboard grid design

Purpose-built for widgets; it borrows the feel of the desktop dock but shares no code with it.

- **Model**: a fixed-column grid (24 columns; cell width = page width / 24, cell height = same, so cells are square and the grid scales with the screen). A tile is `{x, y, w, h}` in cells. **Free placement**: gaps are allowed, the page grows downward and scrolls. Tiles never overlap.
- **Resize**: grab any **corner** (or edge); the rect snaps to whole cells, with a live size badge (`6 × 4`). Minimum size per widget type (e.g. gauge 3 × 3, chart 6 × 4, numeric 2 × 2).
- **Move**: drag by the grip; the tile snaps to cells with a ghost preview showing where it will land, plus alignment guides to neighbouring tile edges.
- **Collisions**: dropping onto occupied cells pushes the tiles underneath downward (gridstack-style) until nothing overlaps. A per-dashboard **Compact** toggle packs tiles upward to remove vertical gaps; off by default so gaps stay where the user put them.
- **Swap**: drop a tile on the *centre* of another to exchange positions (each takes the other's x / y; collisions resolved as above).
- **Split**: drop a tile on the *edge* of another to dock beside it: the target gives up half its width (left / right edge) or height (top / bottom) and the incoming tile takes the freed half, so you can slot a widget into an existing space without resizing anything by hand. If the target is too small to halve, it falls back to push.
- **Adding**: "+ Tile" drops a default-size tile in the first free spot and opens its editor; or drag a rectangle on empty grid space to draw the tile exactly where wanted.
- **Edit mode**: page-level Edit shows the grid dots, grips, corner handles, pencil and ×; undo / redo for layout changes. View mode shows only widgets (and the optional title line).
- **Small screens**: below a width threshold the page ignores x / w and stacks tiles in y-then-x order at full width, so a phone is usable without a separate layout. Kiosk mode can alternatively "fit to screen" (scale the whole grid to the viewport).
- **Layout JSON**: `{"v":1,"cols":24,"compact":false,"tiles":[{"id":1,"x":0,"y":0,"w":6,"h":4,"widget":"gauge","point":"psi","title":"…"}]}`.


## Chart widget

**Series.** One chart tile holds a list of series (up to 8): each is a point, a label, a colour (auto-assigned from a palette, editable), line style (line / step / area) and which Y axis it uses. Edit mode shows a series list with "+ Add point" (the same point picker as other tiles), reorder and remove.

**Axes.** Time on X, with a window chosen per tile (1 m / 5 m / 15 m / 1 h / 6 h / 24 h, default 5 m). Up to two Y axes (left / right) so e.g. pressure and rate share a chart; each axis auto-scales or takes fixed min / max, and shows its units (series with different units must sit on different axes, the editor warns otherwise). Boolean / enum points draw as steps on a thin lane under the plot instead of squeezing the scale.

**Interaction.** Hover crosshair with a tooltip listing every series' value at that time; click a legend entry to hide / show a series; a Pause button freezes the view for inspection (data keeps buffering) with a "Live" button to snap back. BAD-quality spans are drawn as dashed / greyed segments, and gaps in data break the line rather than interpolating.

**History comes from the server.** A browser-only buffer would start empty on every page load, so the pack keeps a rolling history per bound point:
- Sampled on change, thinned to at most one sample per `history_interval` (default 1 s), stored as `(ts, value, quality)` in a ring of `history_minutes` (default 60) per point; memory is capped per connection (default 64 MB) with oldest-first drop, and only points a chart actually uses are retained (the browser tells the pack which points are charted via the layout, the pack reads saved layouts at start).
- On connect the page requests `GET /api/history?points=a,b&since=<ts>&max=<n>`; the server **downsamples to the chart's pixel width** (min / max bucketing so peaks aren't lost) before sending. After that, live changes arrive over the WS and extend the series.
- History is in memory only; it resets when the service restarts (persisting it is a later option; the point of this tile is live monitoring, not a historian).

**Rendering.** `egui_plot` (pure Rust, compiles to wasm) for axes / legend / hover, with the dashboard's own tile chrome and grid around it. Unit-testable pieces in plain Rust: ring buffer, thinning, min / max downsampling, axis auto-scale.

## Write-back (control widgets)

**Wiring.** A dashboard connection has both kinds of binds on the hub: **Output** binds (what tiles display) and **Input** binds (what the browser may write). An Input bind on the dashboard connection is the *only* thing a browser can ever write; the hub then routes that point wherever the user wired it (e.g. to a Modbus output). Nothing writable exists unless the user added the bind. A control tile has a **write target** (an Input bind) and an optional **feedback point** (an Output bind), the usual HMI split between command and status.

**Control widgets.**

| Widget | Sends |
| --- | --- |
| Toggle / switch | on / off values (configurable, e.g. `1` / `0`, `true` / `false`) |
| Button | a value on press; optional "return to" value after N ms (momentary / pulse) |
| Setpoint | number entry with ± step, clamped to min / max, explicit Send |
| Slider | number in range, sends on release (or throttled while dragging, opt-in) |
| Selector | one value from a labelled list (`0 = Auto`, `1 = Manual`) |
| Text entry | a string, optional max length |

**Protocol.** Browser → pack: `{"t":"set","id":17,"point":"pump_cmd","value":1}`. Pack → that browser: `{"t":"ack","id":17,"ok":true}` or `{"t":"ack","id":17,"ok":false,"err":"out of range 0–100"}`. An ack means "accepted into the point store", not "the device did it". The tile shows **pending** after sending and settles when the feedback point equals the commanded value, or shows **failed to confirm** after a timeout (default 3 s); with no feedback point it shows "sent" briefly. Display always comes from the feedback value, never a local echo, so every browser shows the same truth.

**Authority and safety (all enforced in the pack, never trusted from the page).**

- Three levels: **view**, **operate** (may write), **design** (may edit layouts). Connection fields `allow_write` and `allow_edit` (both default off on LAN hosts; loopback defaults on). Optional `operate_password` and `design_password`; login is `POST /api/login` → session cookie, which the WS and REST both check. Plain HTTP means passwords cross the LAN in clear, so the panel says so; TLS (rustls is already in the tree) is a later option.
- The pack re-validates every `set`: the target is a real Input bind of *this* connection, the value type matches, numbers are clamped / rejected against the bind's min / max / allowed list (taken from the bind config, not from the tile), and per-client rate limit (default 10 sets/s) applies. Tile settings in the layout are presentation only.
- **Confirm** option per control ("Are you sure?" dialog) and **hold-to-press** option for buttons (press and hold 1 s), for things like ESD or start / stop.
- **Interlock**: optional `enable when` expression per control, evaluated in the browser with `pb-expr` (compiled to wasm too) over the live points, e.g. `mode == 1 && estop == 0`; a disabled control shows why. The pack evaluates the same expression before accepting, so a modified page can't bypass it.
- A control is disabled (greyed, "no data") while its feedback point is BAD or stale: you can't command what you can't see. Controls are inert while the page is in Edit mode.
- **Audit**: every set is logged via the host log (time, client address, role, point, old → new, accepted / rejected) and the Dashboard panel in Protocol Bench shows a "Recent writes" list from `detail_json`.
- Several browsers at once: last write wins; all see the result through the feedback value.

## Protocol Bench UI side

- `ExtProto::Dashboard` panel in the existing ext framework: connection list, status, peers, "Open dashboard in browser" button, Layouts list (names, delete) read from the pack's `detail_json`.
- Hub Output inspector: the standard point binds (point picker limited to points wired into the hub), plus per-bind label / units / min / max for gauge defaults.
- Add a demo to `demos.rs`: e.g. extend `frac-spread.json` with a Dashboard output and a pre-built "Pump overview" layout.
- Help catalog entry, README table row, `docs/prd.md` + `pack-development.md` updates.

## Phases

1. **Foundation** — `pb-webserve` rlib (move the HTTP / WS accept loop out of `pb-websocket`, add the small router + tests), `pb-websocket` ported onto it; `pb-dash-model` crate; wasm toolchain + build script producing a "hello" page served by an empty `pb-dashboard` pack.
2. **Read-only dashboard** — WS data feed + server-side history buffer, grid in view mode, Numeric / Text / Gauge / Indicator tiles, one layout, Protocol Bench panel + Open button.
3. **Chart widget + editing + multiple dashboards** — chart first (history is already in the pack from phase 2), then the grid edit mode (corner resize, snapped move, swap / split drops, grip / pencil header, undo / redo), tile editor, layout REST + persistence, switcher bar, add / rename / duplicate / delete / default, `#/name`, `?kiosk`.
4. **Polish** — Bar widget, zones / thresholds, quality / stale styling, light / dark, demo project, docs.
5. **Control widgets** — protocol + pack validation, toggle / button / setpoint / slider / selector / text, pending / confirm logic, interlock expressions, roles + passwords, audit list, demo wired back to a device.

## Testing

- Pack: integration tests like the WebSocket pack's — HTTP routes, layout CRUD round-trip (including path-traversal names rejected), snapshot-then-change over WS, only-bound-points leak check, `allow_edit = false` rejects PUT / DELETE.
- Grid module (pure data, native): place, move, corner / edge resize, push-down collision resolution, compact, swap, edge-split (including too-small fallback), first-free-slot search, minimum sizes, small-screen stacking order.
- Model crate: layout JSON round-trip and version migration tests.
- Wasm app: model / widget-layout logic is plain Rust and unit-tested natively; rendering checked by driving the real page in the browser pane, inside the isolated netns / Xvfb setup so it never touches the running service, ports or Modbus Binge.

## Risks

- **Wasm size / build friction** — one-off toolchain setup; checked-in `dist/` keeps normal builds simple.
- **Unauthenticated access on a LAN** — LAN hosts default to read-only until passwords are set (see write-back).
- **Writing to plant equipment** — the pack-side validation, interlocks and audit are the point of that design; control widgets ship last and are off by default.
- **egui on web** is canvas-rendered: no text selection / browser-native accessibility. Acceptable for a display dashboard; the cost of staying all-Rust.

## Decisions made

- egui / WASM front end (all Rust), served by the pack.
- Layouts stored as JSON files in the config dir beside the project.
- Free-placement snapping grid (gaps allowed) with corner resize, plus swap and split drops.

## Open questions (for the end of the pass)

1. Chart history: in memory only (proposed) or persisted across service restarts later?
2. Grid density: 24 columns of square cells is the starting point; adjust after seeing it on a real wall display.

## Phase 1 status (built)

- `pb-webserve` (HTTP helper), `pb-websocket` ported onto it (handshake now off the worker thread), `pb-dash-model` (layout + wire types, builds for wasm), `pb-dash-web` (hello app + WS client), `pb-dashboard` (pack: serves the embedded app, `/api/health`, `/ws` snapshot + changes for Output-bound points only). `cargo run -p pb-dashboard --example serve` runs it standalone on port 18780.
- `scripts/build-dashboard.sh` builds the wasm into `pb-dashboard/dist/` (committed so a normal `cargo build` needs no wasm toolchain). Needs `trunk` + the `wasm32-unknown-unknown` target. The build currently skips `wasm-opt` (its download timed out); the wasm is ~3 MB uncompressed. Add `wasm-opt` / gzip later.
- **Not verified:** the canvas actually drawing. The data path (page → `/ws` → snapshot + changes) was checked from a real browser session, but the built-in browser pane has no WebGL, and Claude in Chrome was not connected, so the egui render is unseen.
- **Risk to settle:** eframe's web build needs WebGL. Kiosk PCs, VMs and thin clients without GPU acceleration may show "WebGL isn't supported" (as the pane did). If wall displays like that matter, we'll need a fallback (a plain-HTML render path) or to document the requirement.
- Confirmed by the user in a real browser: the page draws and shows live points.
- **Wired into the service and UI**: `EngineId::Dashboard`, `ExtProto::Dashboard`, `HubProto::Dashboard` (offered on hub Outputs only). **The node is the whole configuration**: a Dashboard Output node on the hub diagram has a title, a port and an update rate, and nothing else. Its connection is created and kept in step automatically (`sync_dashboard_conns`), there is no Dashboard panel and no connection dropdown, and every point wired into the node by an arrow is exposed to the page automatically (`dashboard_auto_binds`); what to show is chosen on the dashboard page itself. The Generator plant demo has one on `:8780`. Checked end to end in an isolated service.

## Phase 2 + 3 status (built)

- **Grid** (`pb-dash-model::grid`, 12 unit tests incl. a 400-step random stress that must never overlap tiles): 24 columns, free placement, push-down collisions, compact, swap, edge-split, first-free-slot, minimum sizes.
- **Pack** (`pb-dashboard`): `GET /api/dashboards`, `GET|PUT|DELETE /api/dashboards/<name>` (validated, atomic file writes, names cannot escape the folder, 64 dashboards max), `GET /api/history?points=&since=&max=` (in-memory ring per numeric point, thinned to 1/s, min/max downsampling), `allow_edit` / `history_minutes` connection options. The service gives every dashboard node its own folder: `<config dir>/dashboards/<project name>/<connection id>/`.
- **Page** (`pb-dash-web`): widgets Numeric, Text, Gauge (colour zones), Bar, Indicator, Chart (several points, two Y axes, line / step / area, hover readout, clickable legend, server history on load) and Label; Edit mode with grips, eight resize handles per tile, live preview of drops, swap / split drop zones, draw-a-tile on empty space, tile editor with a searchable point picker, undo / redo, compact toggle, auto-save (0.6 s debounce); dashboard switcher with New / Duplicate / Rename / Delete; `#/Name` deep links, `?kiosk`; single-column stack below 640 px.
- **Development loop**: `cargo run -p pb-dash-web -- [--empty] [--read-only]` opens the whole page as a desktop window with synthetic data and an in-memory server, so it can be screenshotted under Xvfb without a browser.
- **Not done yet**: chart pause button, control widgets / write-back and passwords (phase 5), `wasm-opt`, a demo project with a ready-made "Pump overview" layout saved on the server, gzip of the wasm.

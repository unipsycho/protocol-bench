//! Help catalog — every in-app help article, organised by category and tagged for search.
//!
//! Bodies use a small markup the Help panel renders with theme colours:
//!
//! | Markup | Renders as |
//! | --- | --- |
//! | `## Heading` / `### Sub` | section heading (accent) / sub-heading |
//! | `- item` / `  - item` | bullet / nested bullet |
//! | `1. item` | numbered step (badge) |
//! | `> text` | tip callout |
//! | `!> text` | caution callout |
//! | `i> text` | note callout |
//! | ```` ``` ```` | code block (lines until the closing fence) |
//! | `\| a \| b \|` | table row (first row is the header; `\|---\|` rows are skipped) |
//! | `**bold**`, `` `code` ``, `*em*` | inline styles |
//!
//! A blank line ends a paragraph. Keep article ids stable: `related` refers to them.

use egui::Color32;

use crate::theme::Proto;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    GettingStarted,
    Workspace,
    Project,
    Points,
    Hubs,
    Transforms,
    Monitoring,
    Protocols,
    Service,
    Reference,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::GettingStarted,
        Category::Workspace,
        Category::Project,
        Category::Points,
        Category::Hubs,
        Category::Transforms,
        Category::Monitoring,
        Category::Protocols,
        Category::Service,
        Category::Reference,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::GettingStarted => "Getting started",
            Self::Workspace => "Workspace",
            Self::Project => "Projects",
            Self::Points => "Points",
            Self::Hubs => "Hubs & diagram",
            Self::Transforms => "Calc & Agg",
            Self::Monitoring => "Mappings & traffic",
            Self::Protocols => "Protocols",
            Self::Service => "Service & packs",
            Self::Reference => "Reference",
        }
    }

    /// One line for the Help home's category cards.
    pub fn blurb(self) -> &'static str {
        match self {
            Self::GettingStarted => "What Protocol Bench is, the core ideas, and a first bridge step by step.",
            Self::Workspace => "The top bar, menus, panels and docking, themes, settings and the status bar.",
            Self::Project => "Opening, saving and switching projects, what a project file holds, the demo.",
            Self::Points => "The shared values everything flows through: ids, quality and the Points panel.",
            Self::Hubs => "Hubs, the diagram, wiring, Input and Output nodes, starting and stopping hubs.",
            Self::Transforms => "Calculations on a wire (scale, deadband, averages…) and Aggregators.",
            Self::Monitoring => "Check what leaves where in Mappings, and what is live in Traffic.",
            Self::Protocols => "Set up and bind MQTT, Modbus, OPC UA, Streams, HTTP, files, LAS, SNMP, WITS, WebSocket, Sparkplug B, DNP3 and CAN.",
            Self::Service => "Running the service, protocol packs, troubleshooting and security.",
            Self::Reference => "JSON paths, the glossary, and what is not built yet.",
        }
    }

    pub fn color(self) -> Color32 {
        match self {
            Self::GettingStarted => Proto::COPPER,
            Self::Workspace => Proto::INDIGO,
            Self::Project => Proto::FILES,
            Self::Points => Proto::SAGE,
            Self::Hubs => Proto::TEAL,
            Self::Transforms => Proto::OPC,
            Self::Monitoring => Proto::STREAMS,
            Self::Protocols => Proto::MODBUS,
            Self::Service => Proto::SNMP,
            Self::Reference => Proto::LAS,
        }
    }
}

pub struct Article {
    pub id: &'static str,
    pub title: &'static str,
    pub category: Category,
    /// Overrides the category colour (protocol articles use the protocol colour).
    pub accent: Option<Color32>,
    pub tags: &'static [&'static str],
    pub summary: &'static str,
    pub body: &'static str,
    pub related: &'static [&'static str],
}

impl Article {
    pub fn color(&self) -> Color32 {
        self.accent.unwrap_or_else(|| self.category.color())
    }
}

pub fn find(id: &str) -> Option<&'static Article> {
    ARTICLES.iter().find(|a| a.id == id)
}

pub const ARTICLES: &[Article] = &[
    // ───────────────────────────── Getting started ─────────────────────────────
    Article {
        id: "welcome",
        title: "What is Protocol Bench?",
        category: Category::GettingStarted,
        accent: None,
        tags: &["overview", "purpose", "intro", "about"],
        summary: "A native always-on converter that moves values between industrial and oilfield links.",
        body: r#"
**Protocol Bench** moves values between industrial and oilfield links. A value that arrives on one protocol — a serial instrument, a Modbus register, a growing CSV, an MQTT topic — can leave on another, such as an OPC UA server or an MQTT broker.

## Why it exists

Moving a value between a serial instrument, a Modbus device, a log file, an MQTT broker and an OPC UA client usually means a chain of single-purpose tools, a hardware gateway sold per protocol pair, or a plant connectivity server licensed by tag count. Protocol Bench does the small job — a few ports in, a few topics out — in one place.

## Two processes

- **The service** (`pb-service`) is always on. It owns the project, holds the points and runs the protocol packs.
- **The UI** (this window) configures and monitors the service. Closing the UI does not stop your bridges.

## What it speaks today

| Protocol | Role |
|---|---|
| MQTT | Client (subscribe / publish) and embedded broker hub |
| Modbus | TCP and RTU, client (master) and server (slave) |
| OPC UA | Client (browse / subscribe) and embedded server hub |
| Streams / TCP | Serial ports, TCP clients and listeners — raw or framed text |
| HTTP / REST | Poll JSON endpoints, push JSON bodies |
| File follow | Growing CSV and JSON-lines files |
| LAS | LAS 2.0 well logs — follow and write |
| SNMP | v1 / v2c GET polling |
| WITS | WITS level 0 in and out, over TCP or serial |
| WebSocket | JSON client and server |
| Sparkplug B | Host and edge node |
| DNP3 | TCP master: polls and controls |
| CAN | SocketCAN signals, including J1939 PGNs |

> New here? Read **Quick start** next, then **Core concepts**.
"#,
        related: &["quick-start", "concepts", "service-and-ui"],
    },
    Article {
        id: "quick-start",
        title: "Quick start: your first bridge",
        category: Category::GettingStarted,
        accent: None,
        tags: &["setup", "tutorial", "first", "how to", "walkthrough"],
        summary: "Build a working Input → hub → Output route step by step.",
        body: r#"
This walkthrough bridges a Modbus device to MQTT. The same shape works for any pair of protocols.

## 1 · Make sure the service is up

Look at the **status bar** at the bottom. The service chip should read *connected*. If it reads *disconnected*, start the service (see **Running the service**).

## 2 · Add the connections

Connections live in the protocol panels. Bind details live on hub nodes.

1. Open the **Modbus** panel and add a **Client (master)** connection: TCP host and port (or an RTU serial line) and a default unit id.
2. Open the **MQTT** panel and add a broker connection: host, port, client id, then **Apply / Connect**.

## 3 · Create a hub

1. Open the **Hubs** panel and press **New hub** (or right-click the empty canvas).
2. Pick the hub protocol. A hub is one server-capable protocol: Modbus server, MQTT broker or OPC UA server.

## 4 · Add an Input and bind points

1. Add a **Modbus Input** node to the hub and pick your Client connection and a poll interval.
2. Under **REGISTERS → modbus_N**, add binds such as `holding 100 f32`. Each bind becomes a point (`modbus_1`, `modbus_2`, …).
3. Check the **Points** panel — values should arrive with *Good* quality.

## 5 · Add an Output and wire it

1. Add an **MQTT Output** node and pick the broker connection.
2. **Alt-drag** from the Input node onto the Output node to draw a wire. The Input's points are added to the Output automatically (marked *auto*).
3. Edit the publish topics as needed.

## 6 · Save

Use **File → Save project**. The diagram layout is saved with the project.

> Use the **Traffic** panel and the **Mappings** panel to confirm values are flowing and leaving where you expect.
"#,
        related: &["concepts", "wiring", "modbus", "mqtt", "file-menu"],
    },
    Article {
        id: "concepts",
        title: "Core concepts",
        category: Category::GettingStarted,
        accent: None,
        tags: &["overview", "connections", "inputs", "outputs", "points", "hubs", "feeds", "model"],
        summary: "Connections, hubs, Inputs, Outputs, points and wires — and who owns what.",
        body: r#"
## The building blocks

| Thing | What it is | Where you set it up |
|---|---|---|
| **Connection** | A link to a device or service: broker, PLC, endpoint, file, serial port | Protocol panels (MQTT, Modbus, …) |
| **Hub** | A named routing point with one server protocol | Hubs panel |
| **Input** | A node that reads from a connection and creates points | Hub diagram |
| **Point** | A shared value with id, value, quality, source and time | Created by Input binds; listed in Points |
| **Output** | A node that sends chosen points to a connection | Hub diagram |
| **Wire (feed)** | The data path from an Input / Calc / Agg to an Output or another hub | Hub diagram |
| **Calculation** | A single-stream transform on a wire | On a wire |
| **Aggregator** | Combines two or more points into one | On a wire |

## Who owns what

- **Protocol panels own connections only.** Host, port, credentials, serial settings.
- **Hub nodes own binds.** Which topic, register, NodeId, column or curve becomes which point; which point goes out where.
- **Several Inputs and Outputs may share one connection.**

## Rules of thumb

- Data only flows along wires.
- Points are project-wide: every point shows in the Points panel, whatever hub created it.
- Outputs refer to points by id — pick the point instead of retyping the source address.
- To share data between hubs, wire one hub to the other.
"#,
        related: &["data-flow", "points-overview", "hubs-overview"],
    },
    Article {
        id: "data-flow",
        title: "How data flows",
        category: Category::GettingStarted,
        accent: None,
        tags: &["flow", "routing", "pipeline", "direction", "edges"],
        summary: "Inputs → optional transforms → hub → optional transforms → Outputs.",
        body: r#"
Data moves one way through a hub:

```
Input → [Calc / Agg] → Hub → [Calc / Agg] → Output
```

1. An **Input** reads its connection (poll, subscribe, follow or scan) and writes values into **points**.
2. Optional **Calculations** and **Aggregators** sit *on the wire*, transforming values on the way.
3. The **hub** decides which points leave and where. A server hub (Modbus server, OPC UA server, MQTT broker) also serves its chosen points to remote clients.
4. An **Output** sends the chosen points to its connection (publish, write, push, emit or log).

## Values travel with quality

Every point carries a quality: *Good*, *Uncertain* or *Bad*. When a link dies, its points go **Bad** and keep their last value, and the pack retries the link. See **Point quality**.

## Writes back to a device

Outputs can also drive devices: a Modbus Output writes coils and holding registers, so an MQTT message can drive a PLC coil through a hub.

i> Mappings are **type-preserving copies** today. Scaling, offsets and unit conversion are done with a **Calculation** on the wire.
"#,
        related: &["concepts", "calculations", "point-quality"],
    },
    Article {
        id: "service-and-ui",
        title: "Service and UI",
        category: Category::GettingStarted,
        accent: None,
        tags: &["service", "ui", "attach", "quit", "tray", "background", "always on"],
        summary: "Why closing the window does not stop your bridges, and how the UI attaches.",
        body: r#"
Protocol Bench is two programs:

- **`pb-service`** — the always-on converter. It loads the protocol packs, owns `project.json`, holds the points and runs the hubs.
- **`protocol-bench`** — this UI. It shows and edits what the service is running.

## Attach, not restart

The UI opens **whatever project the service is already running** when it has live links; otherwise it opens the project you used last. If the service is down, the UI still opens and reconnects automatically about every 1.5 s, then opens your last project. The **status bar** chip shows *connected* or *disconnected*.

## After a restart or crash

The service remembers the project it last opened (`last-project` in the settings folder) and reopens it when it starts again, even after the process was killed. `PB_PROJECT` still wins when set. If the remembered file was deleted, the service falls back to the default project.

## Switching projects

Opening another project first lists the live hubs it will stop and asks you. The service then stops the old project's hubs and clients before applying the new one.

## Quitting

Closing the UI leaves the service running. When hubs are live, the UI asks:

- **Quit, keep running** — the service keeps bridging in the background.
- **Stop all and quit** — stops hubs, disconnects clients, then closes.
- **Stay** — cancel.

## Tray icon (Linux)

The service shows a tray icon (green dot when hubs are live) listing the project and the running hubs, with **Stop all hubs** and **Quit service**.
"#,
        related: &["running-service", "status-bar", "switching-projects"],
    },
    // ───────────────────────────── Workspace ─────────────────────────────
    Article {
        id: "top-bar",
        title: "The top bar",
        category: Category::Workspace,
        accent: None,
        tags: &["menu", "toolbar", "header", "file", "view", "help", "settings", "theme", "banner"],
        summary: "Logo, File / View / Help menus, hub status strip, theme chip and settings gear.",
        body: r#"
The bar across the top of the window, from left to right:

| Part | What it does |
|---|---|
| **Protocol Bench mark** | Click it to open **About** (version, slogan, Agile Automation). |
| **File** | Open, save, save as, recent projects, quit. |
| **View** | Show / hide each panel, **Show all panels**, **Reset layout**. |
| **Help** | **Help topics** (this panel), **About Protocol Bench**, **Settings**. |
| **Hub status strip** | Live summary of your hubs, point count and quick start / stop. |
| **Theme chip** | The palette swatch — pick one of seven themes. |
| **Settings gear** | Opens the Settings window. |

## Hub status strip

The strip sits in the free space of the bar and reports, for example, *2 hubs running* or *1 of 2 running*, plus what is listening (`MQTT :1883 · OPC UA :4840`). Its control starts every stopped hub or stops every running one. As the window narrows, parts drop out (headline first, then points, then chip details).

> Hover any control for a tooltip. The theme chip's tooltip names the current theme.
"#,
        related: &["file-menu", "panels-layout", "themes", "settings", "starting-hubs"],
    },
    Article {
        id: "file-menu",
        title: "File menu: open and save projects",
        category: Category::Workspace,
        accent: None,
        tags: &["file", "new", "open", "save", "save as", "recent", "quit", "project"],
        summary: "New project…, Open project…, Save, Save as…, Open recent and Quit.",
        body: r#"
| Item | What it does |
|---|---|
| **New project…** | Choose a file name; creates an empty project (no connections, no hubs) and opens it. |
| **Open project…** | Native file dialog; the service switches to the chosen project. A file that is not a valid project is refused and the current project stays open. |
| **Open recent** | Up to 15 recent projects, plus a clear option. |
| **Save project** | Writes the current project (hubs, connections, binds, layout). |
| **Save project as…** | Choose a new file and save there. |
| **Quit** | Close the UI (see **Service and UI** for what happens to live hubs). |

!> Opening a project **stops the live hubs** of the current one. The UI lists them and asks first.

i> The file dialog runs on a worker thread so the window never freezes while it is open.
"#,
        related: &["project-file", "switching-projects", "service-and-ui"],
    },
    Article {
        id: "panels-layout",
        title: "Panels, docking and the side rail",
        category: Category::Workspace,
        accent: None,
        tags: &["layout", "dock", "tabs", "split", "rail", "view", "panels", "reset", "reorder", "drag", "group", "protocols", "pop out"],
        summary: "Drag panel titles to dock, tab or split; toggle panels from the rail or View menu; pick protocols in the Protocols panel.",
        body: r#"
The window is a set of **docked panels**. Arrange them however suits the job.

## Rearranging

- **Drag a panel title** onto another panel to dock it there.
- Drop near an **edge** to split the area; drop in the middle to add it as a **tab**.
- Drag the **dividers** between panels to resize them.

## Reordering tabs

Drag a tab **along its own strip**. A glowing caret shows where it will land between the other tabs; release to drop it there. Dropping a tab on **another** strip inserts it at the caret.

## Moving a whole group of tabs

Grab the **grip** (three bars) at the left of a tab strip, or the empty strip space right of the last tab. The ghost shows every tab you are carrying. Then drop it like a single panel:

| Drop on | Result |
|---|---|
| An edge pad | Docks the whole group on that side |
| The centre **Swap** pill | Swaps the two areas |
| Another tab strip | Merges all the tabs in at the caret |

## Showing and hiding

- The **side rail** on the left has one icon per panel. Click to toggle it.
- **View** lists every panel with a check mark, plus **Show all panels** and **Reset layout**.
- At least one panel always stays open.

## The Protocols panel

Every protocol's connections live in one **Protocols** panel. Pick the protocol from the dropdown at the top:

- Protocols the project already uses are listed first, then the others.
- The dot is the protocol's state (green live, red error, grey idle). The number is how many connections it has.
- Only protocols whose pack the service has loaded are offered. While the service is offline, every protocol is listed so you can still edit the config.

To work on two protocols side by side, click **Pop out**: the current protocol gets its own panel beside Protocols, and you can dock it anywhere. Its **×** button (or **View → … (dock back)**) puts it back in the dropdown.

## The panels

| Panel | Purpose |
|---|---|
| Project | Project summary tiles and hub inventory |
| Hubs | The hub diagram and node properties |
| Points | Every point with value, quality, source, update time |
| Mappings | Read-only list of every route |
| Traffic | Listeners, connected clients and throughput |
| Help | This guide |
| Protocols | Connections for each protocol, chosen from a dropdown |

i> The layout is stored in `~/.config/protocolbench/ui.toml`, separate from the project.
"#,
        related: &["top-bar", "settings"],
    },
    Article {
        id: "themes",
        title: "Themes",
        category: Category::Workspace,
        accent: None,
        tags: &["theme", "colour", "color", "dark", "light", "appearance", "palette"],
        summary: "Seven palettes, chosen from the theme chip on the top bar.",
        body: r#"
Click the **theme chip** (the colour swatch on the right of the top bar) and pick a palette.

| Theme | Feel |
|---|---|
| **Copper** | The default — warm copper accent on dark slate |
| **Ink** | Deep, low-glare dark |
| **Daylight** | The light theme for bright rooms |
| **Harbor** | Cool sea blues |
| **Ember** | Warm reds and oranges |
| **Signal** | High-contrast accent for control rooms |
| **Frost** | Pale, icy tones |

The hub canvas and panel diagrams keep their *scope blue*, mixed with the theme background so wires stay readable in every theme. Each protocol keeps its own colour (MQTT teal, Modbus amber, OPC UA violet, …) across themes.

i> The chosen theme is saved in `ui.toml`, not in the project, so it follows you between projects.
"#,
        related: &["top-bar", "settings"],
    },
    Article {
        id: "settings",
        title: "Settings",
        category: Category::Workspace,
        accent: None,
        tags: &["settings", "preferences", "auto-run", "auto start", "engine host", "gear"],
        summary: "Auto-run embedded hubs, layout reset, About, and the engine host status.",
        body: r#"
Open **Settings** from the gear on the top bar or **Help → Settings**.

## Hubs

**Auto-run embedded hubs** — when enabled, the MQTT broker and OPC UA server hubs start on load (and immediately when you turn this on) if they are configured in their panels. It also applies when you switch projects.

## Layout

- **Reset layout** — puts every panel back in its default place.
- **Show all panels** — reopens any hidden panel.
- **About** — version and credits.

## Engine host

Shows the engine thread name, whether it is *up* or *stopped*, and its uptime.

> The theme is not in Settings — use the theme chip on the top bar.
"#,
        related: &["themes", "starting-hubs", "opcua-server", "mqtt-broker"],
    },
    Article {
        id: "status-bar",
        title: "Status bar",
        category: Category::Workspace,
        accent: None,
        tags: &["status", "connected", "disconnected", "service chip", "messages"],
        summary: "The last action's message and the service connection chip.",
        body: r#"
The strip along the bottom of the window shows:

- **Left** — the latest message: *Saved project*, *Showing Points*, errors from the last action.
- **Service chip** — whether the UI is connected to the service over localhost.

## Service disconnected?

1. Check that the service process is alive: `pgrep -a pb-service`.
2. If you just restarted it, wait a moment — the UI retries about every 1.5 s and re-reads `~/.config/protocolbench/ipc.json`.
3. Only one service should run at a time; the UI attaches to whichever wrote `ipc.json` last.
"#,
        related: &["running-service", "troubleshooting"],
    },
    // ───────────────────────────── Project ─────────────────────────────
    Article {
        id: "project-panel",
        title: "The Project panel",
        category: Category::Project,
        accent: None,
        tags: &["project", "tiles", "inventory", "summary", "hubs", "start", "stop"],
        summary: "Summary tiles and the hub inventory with start / stop controls.",
        body: r#"
The Project panel is the at-a-glance view of the open project.

## Tiles

Counts for **HUBS**, **INPUTS**, **OUTPUTS**, **POINTS**, **CALCS** and **AGGS**. The **HUBS** tile has Start / Stop controls for the embedded MQTT broker and OPC UA server.

## Inventory

One row per hub with its protocol, endpoint and run state. Rows for startable hubs carry their own Start / Stop.

> Starting from here, from the hub's pill on the diagram, from the top-bar strip or by auto-run all behave the same.
"#,
        related: &["starting-hubs", "project-file"],
    },
    Article {
        id: "project-file",
        title: "Project files",
        category: Category::Project,
        accent: None,
        tags: &["project.json", "json", "file", "save", "samples", "version", "ui.toml"],
        summary: "What a project file holds, where samples live, and what is stored elsewhere.",
        body: r#"
A project is **one JSON file**. It holds:

- connections for each protocol (`mqtt.clients[]`, `modbus`, `opcua`, `streams`, `http`, `files`, `las`, `snmp`)
- the embedded broker / server settings
- `hubs` with their Inputs, Outputs, Calculations, Aggregators and binds
- `points[]` and the next point sequence numbers
- the diagram layout

## Stored elsewhere

| File | Holds |
|---|---|
| `~/.config/protocolbench/ui.toml` | Panel layout, theme, recent projects, Settings |
| `~/.config/protocolbench/ipc.json` | How the UI finds the service |
| `<config dir>/file-follow/<source>.json` | File follow byte offsets |

## Samples

Sample projects live in `../protocol-bench-projects/`: `modbus-plant.json`, `modbus-hub.json`, `edge-logger.json` and others.

!> Passwords and tokens are stored **in plain text** in the project JSON (no keyring yet). Treat project files as sensitive.
"#,
        related: &["file-menu", "demo-projects", "security"],
    },
    Article {
        id: "switching-projects",
        title: "Switching projects safely",
        category: Category::Project,
        accent: None,
        tags: &["switch", "open", "stop hubs", "project"],
        summary: "What happens to live hubs and clients when you open another project.",
        body: r#"
1. Choose **File → Open project…** or a recent project.
2. If hubs are live, the UI lists them and asks before going on.
3. The service stops the old project's hubs and disconnects its clients.
4. The new project is applied. With **Auto-run embedded hubs** on, its configured broker and OPC UA server start.

> Save first — unsaved diagram changes are not carried over.
"#,
        related: &["file-menu", "settings"],
    },
    Article {
        id: "demo-projects",
        title: "Demo: Modbus Plant",
        category: Category::Project,
        accent: None,
        tags: &["demo", "sample", "example", "simulator", "plant", "try"],
        summary: "A runnable sample: a fake tank PLC bridged to MQTT and a Modbus server.",
        body: r#"
```
python3 scripts/modbus-plant-sim.py
PB_MODULES=$PWD/modules PB_PROJECT=../protocol-bench-projects/modbus-plant.json cargo run -p pb-service
cargo run
```

## What it shows

- The **Plant** hub polls the fake PLC on `127.0.0.1:5020` (unit 1).
- Values publish to **MQTT**.
- The same points are served by a **Modbus server** on `:1502`.
- An MQTT message on `plant/pump/cmd` drives the PLC's pump coil through a Modbus Output.

i> The sample project shipped in a fresh install has its Modbus connections **disabled**; the demo file enables them.

More samples (`modbus-hub.json`, `edge-logger.json`, with `scripts/field-sim.py` as their SNMP agent and file feeder) are described in the projects folder's README.
"#,
        related: &["quick-start", "modbus", "running-service"],
    },
    Article {
        id: "industrial-demos",
        title: "Industrial demos: frac spread, bottling line, lift station",
        category: Category::Project,
        accent: None,
        tags: &["demo", "pb-sim", "simulator", "frac", "bottling", "factory", "water", "lift station", "wastewater", "web demo", "headless"],
        summary: "Three runnable plant projects with simulated devices for every protocol, plus a generator-only one.",
        body: r#"
`pb-sim` (in this repository, all Rust) plays the field devices; the projects live in `protocol-bench-projects/`.

```
cargo run -p pb-sim -- frac          # or: bottling, water, all
scripts/dev.sh --project ../protocol-bench-projects/frac-spread.json
```

| Project | Hub | Devices | Out |
|---|---|---|---|
| **frac-spread.json** | MQTT (broker :1885) | 4 frac pumps + blender (Modbus TCP 5021–5025), wellhead RTU (DNP3 :20000), pressure-pumping WITS feed (:5101), pump 1 engine (CAN J1939, vcan0, disabled until present) | WITS to the data van (:5100), WebSocket dashboard (:8765), `frac/#` topics, stage LAS log, ESD → RTU CROB, pump 1 rate / enable from `frac/cmd/#` |
| **bottling-line.json** | MQTT (broker :1886) | Line PLC (5031), conveyor / filler VFDs (5032 / 5034), energy meter (5033), robot cell as a Sparkplug B edge node | ISA-95 UNS `plant/bottling/line2/#`, Sparkplug B gateway edge node, WebSocket dashboard (:8766), line run + conveyor speed from MQTT |
| **water-lift-station.json** | OPC UA server (:4855) | Lift station RTU (DNP3 :20001), chlorine analyzer (Modbus TCP 5041) | OPC UA for SCADA, two-way web HMI on WebSocket :8767 (pump commands → DNP3 CROBs) |
| **generator-plant.json** | MQTT (broker :1887) | None — six Generator nodes make the data (every signal type and trigger) | UNS topics `plant/gen/#` |

## Things to try

- Open a pump's Modbus Input to see its register map with tag names, scaling and live values.
- Publish `true` to `frac/cmd/esd` — the RTU trips every pump and rates fall to zero.
- Publish `15` to `frac/cmd/pump1_rate_sp` — pump 1 holds 15 bbl/min regardless of the stage plan.
- Connect a WITS client to `:5100` and watch the data van feed.

## Headless (web demos)

Run the service without the UI: `PB_AUTO_RUN_HUBS=1 PB_MODULES=… PB_PROJECT=… pb-service` starts the embedded broker / OPC UA server too. `pb-sim watch` prints the live point table; `scripts/demo-smoke.sh` checks all four demos end to end (the generator demo needs no simulator and also verifies the broker topics and that the data moves).
"#,
        related: &["demo-projects", "wits", "dnp3", "sparkplug", "websocket", "modbus-register-map"],
    },
    // ───────────────────────────── Points ─────────────────────────────
    Article {
        id: "points-overview",
        title: "Points",
        category: Category::Points,
        accent: None,
        tags: &["points", "values", "store", "ids", "tags"],
        summary: "The shared, project-wide values every Input creates and every Output reads.",
        body: r#"
A **point** is one shared value. All points live in one project-wide store, so the **Points** panel always shows everything.

## Columns in the Points panel

| Column | Meaning |
|---|---|
| **NAME** | The short point id, e.g. `mqtt_3` |
| **VALUE** | The latest value (number, bool or text) |
| **QUALITY** | Good, Uncertain or Bad |
| **SOURCE** | The wire address it came from |
| **UPDATED** | How long ago it last changed |

## Where points come from

A point is created by an **Input bind**. The hub that owns that Input is the point's **origin hub**. Aggregators create derived points (`agg_N`).

## Using points

- Drag points from the Points panel onto an Output's emit list (Streams) or bind list.
- Outputs pick points by id.
"#,
        related: &["point-ids", "point-quality", "output-nodes"],
    },
    Article {
        id: "point-ids",
        title: "Point ids",
        category: Category::Points,
        accent: None,
        tags: &["ids", "naming", "mqtt_n", "modbus_n", "sequence", "auto"],
        summary: "Point ids are assigned automatically per protocol: mqtt_N, modbus_N, …",
        body: r#"
You do not type point ids. Binding a field on an Input assigns the next id for that protocol.

| Input | Id pattern |
|---|---|
| MQTT | `mqtt_N` |
| Modbus | `modbus_N` |
| OPC UA | NodeId → point |
| Streams / TCP | `stream_N` |
| HTTP / REST | `http_N` |
| File follow | `file_N` |
| LAS | `las_N` |
| SNMP | `snmp_N` |
| Aggregator | `agg_N` |

Sequence numbers are stored in the project (`next_point_seq`), so ids stay stable across saves.
"#,
        related: &["points-overview", "input-nodes"],
    },
    Article {
        id: "point-quality",
        title: "Point quality",
        category: Category::Points,
        accent: Some(Proto::SAGE),
        tags: &["quality", "good", "bad", "uncertain", "stale", "retry", "last value"],
        summary: "Good, Uncertain and Bad — and what happens when a link dies.",
        body: r#"
| Quality | Means |
|---|---|
| **Good** | The value came from a live, answering source. |
| **Uncertain** | No data yet, or the source cannot vouch for it. |
| **Bad** | The source failed: link down, timeout, null, per-item error. |

## When a link dies

- Its points turn **Bad** and **keep their last value**.
- The pack **retries** with backoff; points turn Good again when data returns.
- Per-item failures (an SNMP `noSuchInstance`, a LAS `NULL`) mark **only that point** bad.

## Aggregators

An aggregator's quality is **worst-wins** across its members.
"#,
        related: &["points-overview", "aggregators", "troubleshooting"],
    },
    // ───────────────────────────── Hubs ─────────────────────────────
    Article {
        id: "hubs-overview",
        title: "Hubs",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["hub", "routing", "server", "scope", "catalog"],
        summary: "A hub is a named routing point with one server-capable protocol.",
        body: r#"
A **hub** groups one site or flow: its Inputs, Calculations, Aggregators and Outputs sit together.

## Hub protocols

A hub is always **one protocol**, and only server-capable protocols can be hubs:

| Hub | Serves |
|---|---|
| **Modbus server** | Chosen points as registers to remote masters |
| **MQTT broker** | An embedded broker (rumqttd) on the LAN |
| **OPC UA server** | A catalog of points under `Objects/Points` |

Input and Output nodes on a hub may use **any** protocol.

## What a hub does

- **Scope** — its Outputs and catalog pick from its own points or points wired in from another hub.
- **Server role** — it publishes only the points you choose.

A hub does not hold values itself. It decides which points leave and where.

> Many projects need a single hub. Add more when you have several conversion points.
"#,
        related: &["diagram", "wiring", "starting-hubs", "multi-hub"],
    },
    Article {
        id: "diagram",
        title: "Navigating the hub diagram",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["diagram", "canvas", "zoom", "pan", "select", "delete", "shortcuts", "keyboard", "mouse"],
        summary: "Zoom, pan, select, delete and the right-click menus.",
        body: r#"
Each hub is a node with its Input and Output nodes attached. Wires show what feeds what.

## Mouse and keys

| Action | How |
|---|---|
| Zoom | Mouse wheel |
| Pan | Middle-drag or **Space**-drag |
| Move a node | Drag its body |
| Box-select | Drag on empty canvas |
| Select a wire | Click it |
| Delete | **Delete** / **Backspace** removes selected nodes or the selected wire |
| Menus | Right-click a node, wire or empty canvas |
| Undo | Steps back through deletes and moves |

## Properties

Click a node to show its properties. The toggle switches the properties area between **below the diagram** and **on the right**. Select a single node to edit it.

## Right-click menus

- **Empty canvas** — New hub, Add calculation / aggregator at the click.
- **Node** — modify, open the matching protocol panel, delete.
- **Wire** — Insert calculation, Insert aggregator, delete.

> Hover the **Navigation (?)** badge on the canvas for a quick reminder of all of this.
"#,
        related: &["wiring", "input-nodes", "output-nodes"],
    },
    Article {
        id: "wiring",
        title: "Wiring nodes",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["wire", "feed", "alt-drag", "connect", "edge", "rewire", "link"],
        summary: "Alt-drag or drag a port edge to draw a feed; grab a wire end to rewire it.",
        body: r#"
A **wire** (feed) runs from an Input, Calc or Agg to an Output, or to another hub. Data only flows along wires.

## Draw a wire

- **Alt-drag** from an Input, Calc or Agg onto an Output, or
- drag from a node's **port edge** (left port ← Hub / Input, right port → Output). Either direction works.

Wiring an Input to an Output **adds its points to the Output automatically** (marked *auto*).

## Rewire

Grab a cyan **wire end** and drop it on another node.

## Remove

Click a wire, then press **Delete**. Nodes stay when you delete a wire.

## Many to one

One Output accepts many inbound wires. Many Inputs can share one connection.
"#,
        related: &["diagram", "output-nodes", "calculations"],
    },
    Article {
        id: "input-nodes",
        title: "Input nodes",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["input", "source", "bind", "binds", "poll", "subscribe"],
        summary: "Inputs pick a connection and bind fields into points.",
        body: r#"
An **Input** reads one connection and turns chosen fields into points.

## Setting one up

1. Add the Input to a hub (right-click the hub or use the hub's add menu) and choose its protocol.
2. Pick a **connection** created in that protocol's panel.
3. Bind fields. Each bind creates a point with the next id.

| Protocol | You bind | Helper |
|---|---|---|
| MQTT | Topic + optional JSON path | Topic tree — click a leaf |
| Modbus | Area, address, type, byte order | Typed bind row |
| OPC UA | NodeId | Browse tree — click a Variable |
| Streams | Field of a framed line | LIVE / SAMPLE preview chips |
| HTTP | JSON path in the response | **Fetch preview** then click a path |
| File follow | CSV column or JSON path | Column chips from the file |
| LAS | Curve mnemonic | Curve list from the file header |
| SNMP | OID | OID bind rows |

The node title shows its bound **point count**.

> Many Input nodes may share one connection, each with its own binds.
"#,
        related: &["output-nodes", "point-ids"],
    },
    Article {
        id: "output-nodes",
        title: "Output nodes and the point picker",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["output", "publish", "write", "emit", "point picker", "show all project points", "auto"],
        summary: "Choose which points an Output sends, automatically from wires or by hand.",
        body: r#"
An **Output** sends a chosen set of points to its connection.

## Choosing points

- **Wired** — wiring an Input / Calc / Agg into the Output adds its points (marked *auto*). Edit topics or targets as needed.
- **By hand** — add a bind and pick a point.

The point picker lists the hub's own points, points wired in from other hubs, and points no hub owns yet.

!> **Show all project points** lets you pick from every point. Use it only when you really mean to reach into another hub — wiring hubs together is the supported way.

## What each Output does

| Protocol | Sends |
|---|---|
| MQTT | Publishes each point to a topic (QoS, retain) |
| Modbus | Writes coils / holding registers on a Client connection |
| OPC UA | Writes NodeIds |
| Streams | Emits a text line built from points |
| HTTP | Pushes a JSON body (point → JSON field) |
| LAS | Logs rows of curves to a LAS file |

File follow and SNMP are Input only.
"#,
        related: &["wiring", "input-nodes", "mappings"],
    },
    Article {
        id: "starting-hubs",
        title: "Starting and stopping hubs",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["start", "stop", "run", "auto-run", "broker", "server", "pill"],
        summary: "Start or stop embedded hubs from the diagram, Project panel, top bar or tray.",
        body: r#"
Embedded server hubs (MQTT broker, OPC UA server) can be started and stopped from several places — all equivalent:

- the hub's **pill** on the diagram (**Start this hub** / **Stop this hub**)
- the **Project** panel's HUBS tile and inventory rows
- the **hub status strip** on the top bar (start all / stop all)
- the MQTT and OPC UA panels
- the **tray icon** (Stop all hubs)

## Automatic start

Turn on **Settings → Auto-run embedded hubs** to start configured hubs on load.

## Run timers

Each hub shows *up 12:41* while running, or *stopped 3m ago* after a run this session.

i> A Modbus **Server connection** runs only when a Modbus hub picks it.
"#,
        related: &["settings", "project-panel", "mqtt-broker", "opcua-server", "modbus-server"],
    },
    Article {
        id: "multi-hub",
        title: "Multiple hubs",
        category: Category::Hubs,
        accent: Some(Proto::TEAL),
        tags: &["multi-hub", "hub to hub", "cross hub", "share", "feed"],
        summary: "Wire one hub into another to share points across sites or flows.",
        body: r#"
Use more than one hub when there is more than one conversion point — for example, a Plant hub that serves Modbus and an Edge hub that brokers MQTT.

## Sharing between hubs

**Wire one hub's node to the other.** Points carried by that wire become available to the receiving hub's Outputs and catalog.

> A good bench test: scripts feed hub A's Outputs, and hub B imports those outputs. With bi-directional protocols, exercise many-to-many flows.
"#,
        related: &["hubs-overview", "wiring", "output-nodes"],
    },
    // ───────────────────────────── Transforms ─────────────────────────────
    Article {
        id: "calculations",
        title: "Calculations",
        category: Category::Transforms,
        accent: None,
        tags: &["calc", "calculation", "scale", "offset", "deadband", "average", "rate", "hysteresis", "clamp", "unit convert", "expression", "formula", "transform"],
        summary: "Single-stream transforms that sit on a wire: scale, deadband, averages, and more.",
        body: r#"
A **Calculation** transforms **one** stream on a wire: one source point in, one value out. It never combines several points — that is an Aggregator's job.

## Adding one

Right-click a wire → **Insert calculation**, or select a wire and use **Add calculation**. Pick a type in the help guide to pre-fill its config. **LIVE PREVIEW** shows input → output for a chosen source point.

## Types

| Type | Does | Example |
|---|---|---|
| **Scale / offset** | y = (x × scale) + offset — raw counts → engineering units | `scale = 0.1 · offset = −40` |
| **Unit convert** | °C ↔ °F, psi ↔ kPa, … | `from = °C · to = °F` |
| **Clamp** | Hold the value inside a hard min / max | `min = 0 · max = 100` |
| **Deadband** | Pass only changes larger than the band | `band = ±0.5` |
| **Rolling average** | Mean over the last N samples or a time window | `window = 10 s` |
| **EMA** | Exponential moving average — faster than a plain roll | `alpha = 0.2` or `tau = 5 s` |
| **Rate of change** | Per second / minute / hour | `units = /s` |
| **Hysteresis** | Two thresholds so digital outs do not chatter | `on ≥ 80 · off ≤ 70` |
| **Min / max hold** | Remember peak or valley until reset or window expiry | `mode = max · hold = 30 s` |
| **Time-window filter** | Pass, block or average inside a sliding window | `window = 60 s · mode = avg` |
| **Boolean compare** | Compare to a setpoint → true / false | `x > 50` |
| **Expression** | Free formula over this stream | `(x - 4) * 6.25` |

## Expressions

Use `x` (or `A`) for the edge value. Operators `+ − × ÷ ( )`. Functions: `min`, `max`, `abs`, `clamp`, `sqrt`.
"#,
        related: &["aggregators", "calc-vs-agg", "wiring"],
    },
    Article {
        id: "aggregators",
        title: "Aggregators",
        category: Category::Transforms,
        accent: None,
        tags: &["agg", "aggregator", "sum", "mean", "min", "max", "median", "count", "majority", "window", "combine"],
        summary: "Combine two or more member points into one derived point agg_N.",
        body: r#"
An **Aggregator** combines **N ≥ 2** member points into one derived host point `agg_N`.

## Modes

| Mode | Result |
|---|---|
| sum, mean, min, max, median | Arithmetic over members |
| count | Number of members with values |
| first, last | The first / last member's value |
| any-good | True if any member is Good |
| majority | The most common value |

## Window

- **None (current)** — use the latest value of each member.
- **Last N samples** — 5, 10, 20, 50.
- **Last T seconds** — 5 s, 10 s, 30 s, 60 s.

## Quality

Member quality is **worst-wins**: one Bad member makes the aggregate Bad.

> Use **Add member** to add points, and watch **LIVE PREVIEW** — it needs at least two members with live values.
"#,
        related: &["calculations", "calc-vs-agg", "point-quality"],
    },
    Article {
        id: "calc-vs-agg",
        title: "Calculation or Aggregator?",
        category: Category::Transforms,
        accent: None,
        tags: &["calc", "agg", "choose", "difference"],
        summary: "One stream in → Calculation. Several points combined → Aggregator.",
        body: r#"
| | Calculation | Aggregator |
|---|---|---|
| Inputs | One stream | Two or more points |
| Output | The transformed stream | A new point `agg_N` |
| Typical use | Scale, filter, debounce, convert | Totals, averages, voting |
| Quality | Follows the source | Worst member wins |

Both sit **on a wire**, not as free-floating nodes.
"#,
        related: &["calculations", "aggregators"],
    },
    // ───────────────────────────── Monitoring ─────────────────────────────
    Article {
        id: "mappings",
        title: "Mappings",
        category: Category::Monitoring,
        accent: None,
        tags: &["mappings", "routes", "point", "from", "to", "audit"],
        summary: "A read-only list of every route in the project.",
        body: r#"
The **Mappings** panel lists every route, derived from hub Output binds:

| Column | Meaning |
|---|---|
| **POINT** | The point id |
| **FROM** | Where the point comes from |
| **TO** | Where it leaves |

Use it to check what leaves where without opening each hub.

i> The list is read-only. To add a mapping, add a bind on a hub **Output** node.
"#,
        related: &["output-nodes", "traffic"],
    },
    Article {
        id: "traffic",
        title: "Traffic",
        category: Category::Monitoring,
        accent: None,
        tags: &["traffic", "clients", "listeners", "throughput", "monitor", "debug"],
        summary: "Listeners, connected clients and point throughput.",
        body: r#"
The **Traffic** panel shows the live side of your project:

- **Listeners** — each embedded hub (MQTT broker, OPC UA server, Modbus server) and its address.
- **CONNECTED CLIENTS** — who is attached to each.
- **Throughput** — how fast points are moving.

It is not a message trace. To see raw values, use the **Points** panel, the MQTT topic tree or a Streams **LIVE SCAN** preview.

> Use Traffic to confirm an Input is actually receiving and that a remote client really connected.
"#,
        related: &["mappings", "points-overview", "troubleshooting"],
    },
    // ───────────────────────────── Protocols ─────────────────────────────
    Article {
        id: "mqtt",
        title: "MQTT client",
        category: Category::Protocols,
        accent: Some(Proto::MQTT),
        tags: &["mqtt", "broker", "topic", "subscribe", "publish", "tls", "websocket", "wss", "qos", "retain", "json"],
        summary: "Subscribe to topics as points and publish points to topics.",
        body: r#"
## MQTT panel — connections

| Field | Notes |
|---|---|
| **Label** | Your name for the connection |
| **Host / Port** | Broker address (1883 plain, 8883 TLS typical) |
| **Transport** | `tcp`, `tls`, `ws` or `wss` |
| **Client ID** | Optional; tick **Unique** for a fresh random id on each Connect |
| **Anonymous login** | Untick to enter a username and password |
| **Apply / Connect** | Applies changes and connects |

### Wide subscribe

Optional broker filters (e.g. `pb/#`, `+` and `#` wildcards allowed) for traffic and bring-up. They feed the **topic tree** but do **not** create points — only hub Input paths become points.

## Hub Input

1. Pick the MQTT connection.
2. In the topic tree, **click a leaf** to bind it (auto `mqtt_N`), or type a topic / JSON path.
3. JSON payloads: bind a path such as `data.temp` or `$.a[0]`.

## Hub Output

Pick a connection, then choose points and their publish topics (**OUTPUT BINDS — point → publish topic**). One Output publishes many points. Wiring fills topics automatically.

!> Passwords are stored in the project JSON (no keyring yet).
"#,
        related: &["mqtt-broker", "input-nodes", "output-nodes", "json-paths"],
    },
    Article {
        id: "mqtt-broker",
        title: "MQTT broker hub (embedded)",
        category: Category::Protocols,
        accent: Some(Proto::MQTT),
        tags: &["mqtt", "broker", "embedded", "rumqttd", "hub", "lan", "listen"],
        summary: "Run Protocol Bench's own MQTT broker on the LAN.",
        body: r#"
The **embedded hub broker** is a single project-wide MQTT broker (rumqttd) that listens on your LAN.

1. In the **MQTT** panel, open **Embedded hub broker**.
2. Set the listen address and port.
3. Press **Start broker** — the panel shows *Listening on host:port*.

Create an **MQTT hub** to route points through it. Clients on the LAN connect to it like any broker; a *Local broker* connection lets Protocol Bench's own Inputs and Outputs use it.

> Turn on **Settings → Auto-run embedded hubs** to start it when the project loads.
"#,
        related: &["mqtt", "starting-hubs", "hubs-overview"],
    },
    Article {
        id: "modbus",
        title: "Modbus client (master)",
        category: Category::Protocols,
        accent: Some(Proto::MODBUS),
        tags: &["modbus", "tcp", "rtu", "serial", "plc", "master", "client", "registers", "coil", "poll", "unit id"],
        summary: "Poll PLC registers into points and write points back to coils and registers.",
        body: r#"
## Modbus panel — Client connection

| Field | Notes |
|---|---|
| **Name** | e.g. *Plant PLC* |
| **Transport** | **TCP client** (host:port) or **RTU (serial)** |
| **Serial line** | RTU only: port path, baud, data bits, parity, stop bits |
| **Default unit** | Unit / slave id used unless a bind overrides it |
| **Timeout (ms)**, retries | How long to wait and how often to retry |

## Hub Input — register map → modbus_N

Pick a Client connection and a poll interval, then build the **register map** — the list of registers to poll. Each register gets its own point (`modbus_1`, `modbus_2`, …). See **Modbus register map editor** for the toolbar, templates and CSV import.

Each row is **area**, **address**, **type**, optional **byte order**, optional **unit id** and optional **scale / offset** (raw counts → engineering units). Polling is batched: neighbouring registers share one read request, and the summary line shows how many requests one poll needs.

## Hub Output — points → registers

Pick a Client connection and add coils / holding registers; pick the **host point** each one writes. A point change is written straight away (scaled back to raw counts when the row has a scale). Writing one `bitN` of a holding register does a read-modify-write.

## Failures

A dead link turns its points **Bad**; the pack retries with backoff.
"#,
        related: &["modbus-register-map", "modbus-types", "modbus-server", "streams"],
    },
    Article {
        id: "modbus-types",
        title: "Modbus areas, types and byte order",
        category: Category::Protocols,
        accent: Some(Proto::MODBUS),
        tags: &["modbus", "data types", "byte order", "endian", "word swap", "abcd", "cdab", "float", "f32", "bits", "string"],
        summary: "Which areas hold what, the supported data types, and abcd / cdab / badc / dcba.",
        body: r#"
## Areas

| Area | Number | Holds | Write? |
|---|---|---|---|
| **Coil** | 0x | bool | Yes |
| **Discrete input** | 1x | bool | No |
| **Input register** | 3x | 16-bit words | No |
| **Holding register** | 4x | 16-bit words | Yes |

## Types

| Type | Registers |
|---|---|
| `bool` | Coil / discrete only |
| `u16` `i16` | 1 |
| `u32` `i32` `f32` | 2 |
| `u64` `i64` `f64` | 4 |
| `bit0` … `bit15` | One bit of a register |
| `strN` | ASCII over N registers |

## Byte / word order

| Order | Means |
|---|---|
| `abcd` | Big-endian (Modbus standard) |
| `cdab` | Word swap — common on many PLCs |
| `badc` | Byte swap |
| `dcba` | Little-endian |

> A float that reads as nonsense (like 1.2e-38) usually needs `cdab`.

!> Coil and discrete areas hold `bool` only. A bind that runs past address 65535 is rejected.

## Scale and offset

A numeric register can carry **scale** and **offset**: `value = raw × scale + offset`. A drive that reports current in tenths of an amp uses scale `0.1`; a sensor with a -40 °C zero uses offset `-40`. Writes divide it back out and round to whole counts for integer types. Leave scale `1` / offset `0` for a type-preserving copy.
"#,
        related: &["modbus", "calculations"],
    },
    Article {
        id: "modbus-register-map",
        title: "Modbus register map editor",
        category: Category::Protocols,
        accent: Some(Proto::MODBUS),
        tags: &["modbus", "register map", "registers", "template", "csv", "import", "export", "modicon", "40001", "block", "scale", "tag"],
        summary: "Build a list of registers visually: blocks, device templates, CSV import, Modicon addresses.",
        body: r#"
The same editor appears on a hub **Modbus Input** (registers polled into points), a hub **Modbus Output** (points written to the device) and a **Server** connection's register map (points served to masters).

## Toolbar

| Button | Does |
|---|---|
| **+ Register** | Adds the next register after the last one (same area and type, next free address) and opens it for editing |
| **+ Block** | Adds N consecutive registers of one type — `Temp 1 … Temp 8` at 100, 102, 104 … |
| **Templates** | Ready-made maps: power meter, VFD status / command, tank transmitter, frac pump, flow computer, pump commands |
| **Import** | Paste CSV (from a device manual or spreadsheet); append or replace |
| **Copy CSV** | Copies the map to the clipboard |
| **Sort** | Orders by area, then address |
| **0-based / 40001** | Shows protocol addresses, or Modicon references |

## The map

- The **memory strip** draws one bar per area with the used addresses; click a segment to select its register. On a server map, overlapping registers turn red.
- Each table row shows a status dot (live quality, red for a problem), tag name, area badge (0x / 1x / 3x / 4x), address, type and byte order, scaling, point and live value with units.
- Click a row to open its **edit card**: name, area, address, unit id, type (bit / string sizes), byte order, scale, offset, units and the point. Hover a row for its Modicon reference and live point.

## CSV columns

```
name,area,address,type,order,unit,scale,offset,units,point
Discharge pressure,holding,0,f32,cdab,0,1,0,psi,
Motor current,holding,3,u16,abcd,0,0.1,0,A,
```

The header is optional and may use other names (`tag`, `ref`, `datatype`, `gain`). Leave the area blank and give a Modicon reference (`40101`) to have it worked out. Common type names (`float`, `int16`, `dword`) are understood.

> Modicon `40001` is holding register **0** on the wire. If a device manual's numbers are one off, switch the address style and compare.
"#,
        related: &["modbus", "modbus-types", "modbus-server"],
    },
    Article {
        id: "modbus-server",
        title: "Modbus server hub (slave)",
        category: Category::Protocols,
        accent: Some(Proto::MODBUS),
        tags: &["modbus", "server", "slave", "hub", "register map", "listen", "1502"],
        summary: "Serve host points as Modbus registers to remote masters.",
        body: r#"
A **Server connection** is a Modbus server Protocol Bench hosts — TCP listen or RTU slave — **with its own register map**.

## Setting one up

1. In the **Modbus** panel, add a **Server (slave)** connection: TCP listen address / port (e.g. `0.0.0.0:1502`) or an RTU serial line, and a unit id.
2. Fill the **REGISTER MAP** with the register editor: which host point is served at which area / address / type / order (and optional scale — masters read raw counts, points keep engineering units).
3. Create a **Modbus hub** and pick this Server connection. A server runs **only when a hub uses it**.

**Auto-map wired points** (on the hub, on for new hubs) skips most of step 2: every point on an arrow into the hub gets a row in the server's register map as an **f32 holding register**, packed from the **from address** you set (0 = Modicon 40001), around any rows you made yourself. Change an auto row's address, type or scaling in the register map and it sticks; delete it to leave that point out. Cutting the arrow removes its row; turning the option off keeps the rows as ordinary ones.

## Behaviour

- Point changes update the register image; master writes update the points.
- Unmapped addresses return **exception 2**.
- TCP servers answer the configured unit and 255; RTU slaves answer their unit and accept unit-0 broadcast writes.

!> An empty map means every request gets an exception.
"#,
        related: &["modbus", "modbus-types", "hubs-overview"],
    },
    Article {
        id: "opcua",
        title: "OPC UA client",
        category: Category::Protocols,
        accent: Some(Proto::OPC),
        tags: &["opc ua", "opcua", "client", "browse", "nodeid", "subscription", "security", "endpoint"],
        summary: "Browse plant UA servers and subscribe to NodeIds as points.",
        body: r#"
## OPC UA panel — External UA clients

| Field | Notes |
|---|---|
| **Label** | e.g. *Plant UA* |
| **Endpoint** | `opc.tcp://host:4840/...` |
| **Policy / Mode** | Security policy and mode (`None` by default) |
| **Auth** | Anonymous or username / password |
| **Trust** | How the server certificate is trusted |
| **Subscription interval** | Sampling interval in ms |
| **Auto connect** | Connect on load |

## Browse

**Browse Objects** walks *Objects → folders → variables*. Expand folders to load children; click a **Variable** to select its NodeId.

## Hub Input — NODE BINDS

Pick the client, then click a Variable in **BROWSE** to bind it, or paste a NodeId manually (e.g. `ns=2;s=Tank.Level`).

## Hub Output

Bind points to NodeIds to write values.
"#,
        related: &["opcua-server", "input-nodes"],
    },
    Article {
        id: "opcua-server",
        title: "OPC UA server hub",
        category: Category::Protocols,
        accent: Some(Proto::OPC),
        tags: &["opc ua", "opcua", "server", "hub", "catalog", "certificate", "pki", "sign&encrypt", "anonymous"],
        summary: "Expose chosen points as OPC UA variables under Objects/Points.",
        body: r#"
Protocol Bench can listen as an **OPC UA server** and expose the points named by the OPC UA hub's **catalog** under `Objects/Points`.

## Embedded UA server hub settings

| Field | Notes |
|---|---|
| **Name**, **App URI** | Server identity |
| **Bind / Port** | Listen address (4840 is standard) |
| **Allow anonymous** | Let clients connect without credentials |
| **Sign&Encrypt endpoint** | Also offer Basic256Sha256 Sign&Encrypt beside None |
| **Auto-generate cert** | Create a server certificate on first start |
| **Auto-trust clients** | Accept client certificates automatically |
| **PKI dir** | Where certificates are kept |

## The catalog

On the OPC UA hub, **OPC UA CATALOG** picks which points are published. Tick **Expose unmapped points** to include points without a catalog name (internal names).

!> Anonymous login on a non-loopback bind exposes the server to the whole LAN.

> Use the **Connectivity checklist** in the panel when a client cannot connect.
"#,
        related: &["opcua", "starting-hubs", "security"],
    },
    Article {
        id: "streams",
        title: "Streams / TCP",
        category: Category::Protocols,
        accent: Some(Proto::STREAMS),
        tags: &["streams", "serial", "tcp", "listener", "client", "com port", "baud", "ascii", "text", "instrument"],
        summary: "Serial ports, TCP clients and TCP listeners for text and byte streams.",
        body: r#"
The **Streams / TCP** panel holds **transport only** — framing, parsers and point binds live on hub nodes.

## Endpoint kinds

| Kind | Fields |
|---|---|
| **Serial** | Port path (`/dev/ttyUSB0`, `COM3`), baud, data bits, parity, stop bits |
| **TCP client** | Host and port to dial |
| **TCP listener** | Address and port to accept peers on |

!> TCP listeners **broadcast** each Output emit to every connected peer.

## Hub Input — SCAN → stream_N

1. Pick the endpoint and set framing and parser (see **Framing and parsing**).
2. Watch the **LIVE** line or paste a **SAMPLE** line for offline authoring.
3. Click a preview field chip (or drag it onto **Binds → Points**). The host assigns `stream_N`.

## Hub Output — EMIT

Drag points from the Points panel onto the emit list, reorder them, and preview the outgoing line.
"#,
        related: &["streams-framing", "modbus"],
    },
    Article {
        id: "streams-framing",
        title: "Streams framing and parsing",
        category: Category::Protocols,
        accent: Some(Proto::STREAMS),
        tags: &["framing", "eol", "delimiter", "parser", "csv", "tsv", "regex", "fixed width", "timestamp", "idle", "transparent"],
        summary: "How bytes become lines (framing) and lines become fields (parser).",
        body: r#"
## Framing

| Mode | Behaviour |
|---|---|
| **Delimited** | Split on an end-of-line marker |
| **Transparent** | Byte pipe, no framing |

### End of line

| EOL | Splits on |
|---|---|
| **LF** | `\n` |
| **CRLF** | `\r\n` |
| **CR** | `\r` |
| **Custom** | Your terminator string |
| **Idle time only** | A gap of *Idle (ms)* with no bytes |
| **EOL or idle** | Whichever comes first |

## Parser

| Parser | Example line → fields |
|---|---|
| **Whole line** | `12.5,OK` → `12.5,OK` |
| **CSV** | `12.5,"OK",tank` → `12.5` · `OK` · `tank` |
| **TSV** | tab separated |
| **Whitespace** | `12.5  OK   tank` → 3 fields |
| **Fixed width** | Widths `8,12,6` → three columns |
| **Regex** | Capture groups become fields |

## Timestamp

**None**, **Receive time**, **ISO-8601 field**, **Epoch ms field**, or a **Custom format**.

> Paste a real line into **SAMPLE** first — the preview chips show exactly what the parser makes of it.
"#,
        related: &["streams"],
    },
    Article {
        id: "http",
        title: "HTTP / REST",
        category: Category::Protocols,
        accent: Some(Proto::HTTP),
        tags: &["http", "rest", "json", "api", "poll", "push", "bearer", "basic", "headers", "url"],
        summary: "Poll JSON endpoints into points and push points as JSON bodies.",
        body: r#"
## HTTP / REST panel — clients

| Field | Notes |
|---|---|
| **Name** | e.g. *Local API* |
| **Base URL** | `https://api.example.com/v1` |
| **Auth** | None, Basic (`Basic {b64}`) or Bearer (`Bearer {token}`) |
| **Timeout (s)** | Per request |
| **Default headers** | Sent with every request (**Add header**) |

## Hub Input — JSON path → http_N

1. Pick the client, set the **method** (GET by default) and **path**, and a poll interval.
2. Press **Fetch preview** — the client fetches Base URL + Path into the sample.
3. **Click a path** in the sample to bind it. You can also paste a JSON body by hand.

## Hub Output — point → JSON field

Pick the client, method (POST by default) and path, then map each point to a JSON field in **BODY BINDS**.

!> Put paths in the bind list, not in the sample box — the sample expects a JSON *body*.
"#,
        related: &["json-paths", "security"],
    },
    Article {
        id: "file-follow",
        title: "File follow (CSV / JSON lines)",
        category: Category::Protocols,
        accent: Some(Proto::FILES),
        tags: &["file", "csv", "json lines", "jsonl", "tail", "log", "follow", "rotation", "offset", "meter"],
        summary: "Follow growing CSV or JSON-lines files into points without rereading rows.",
        body: r#"
## File follow panel — sources

| Field | Notes |
|---|---|
| **Path** | Read on the machine running the **service** |
| **Parser** | CSV or JSON lines |
| **Delimiter / Header** | CSV: delimiter; header *Auto-detect* (skips test / file info lines above the column names, finds the names row and where data starts — handles units lines under the names), *Names on line N*, or *None* (numbered columns). The detected layout and preamble show under the field. |
| **Poll (ms)** | How often to check for new data |
| **Start at** | Beginning of file, or End of file (new rows only) |
| **Rotation** | *Reopen when the file is replaced or truncated* |

## Hub Input — COLUMNS / PATHS → file_N

Pick a source and click a **column chip**. CSV binds use a header name (or 0-based index); JSON lines use a dotted path such as `data.temp`. Chips show the latest row's values, even before any bind exists.

## How following works

- Only **complete lines** are read; a half-written row waits for its newline.
- The byte offset is saved, so a restart **catches up** on rows written while the service was down — and never rereads seen rows.
- A truncated or replaced file restarts from byte 0 with a fresh header.
- A missing file turns its points Bad (last value held) until it reappears.

i> One physical line = one row. Multi-line quoted CSV fields are not supported. File follow is Input only.
"#,
        related: &["las", "json-paths", "point-quality"],
    },
    Article {
        id: "las",
        title: "LAS well logs",
        category: Category::Protocols,
        accent: Some(Proto::LAS),
        tags: &["las", "well log", "oilfield", "curves", "depth", "dept", "time", "mnemonic", "frac"],
        summary: "Follow LAS 1.2 / 2.0 files into points and write points to LAS logs.",
        body: r#"
## LAS panel — endpoints

| Field | Notes |
|---|---|
| **Name** | e.g. *Well A* |
| **File path** | On the service machine |
| **Role** | **Incoming (follow)** or **Outgoing (write)** |
| **Grow with file** | Follow appended rows; off loads the file once |

## Incoming — CURVES → las_N

Pick the endpoint and bind curve mnemonics. The curve list comes from the file header (`~V ~W ~C ~A`) even before any bind. The file's `NULL` value makes that curve Bad (last value held). Offsets survive restarts and rotation.

!> `WRAP YES` (multi-line rows) is refused with a clear error.

## Outgoing — point → curve

Bind points to curves and choose the index:

- **depth** — needs a `DEPT` bind; one row each time depth changes, using the latest value of every other curve.
- **time** — at most one row per second, `TIME` in epoch seconds.

A missing or empty file gets a fresh LAS 2.0 header. An existing file is appended to only when its curves match exactly.
"#,
        related: &["file-follow"],
    },
    Article {
        id: "snmp",
        title: "SNMP",
        category: Category::Protocols,
        accent: Some(Proto::SNMP),
        tags: &["snmp", "oid", "community", "v1", "v2c", "switch", "network", "poll", "get"],
        summary: "Poll SNMP v1 / v2c agents and bind OIDs as points.",
        body: r#"
## SNMP panel — targets

| Field | Notes |
|---|---|
| **Name** | e.g. *Core switch* |
| **Host / Port** | Agent address (161 standard) |
| **Version** | v1 or v2c (v3 later) |
| **Community** | e.g. `public` |
| **Timeout (s)** | Per request |

The status reads *Agent answering*, *No response* or *Idle — no hub SNMP Input uses this target*.

## Hub Input — OIDS → snmp_N

Pick a target and poll interval, then bind OIDs such as `1.3.6.1.2.1.1.3.0`.

## Value types

Counter / Gauge / TimeTicks / Counter64 become integers; OctetStrings become text (hex when not printable); IpAddress and OID become text.

## Failures

Requests are batched (10 per request, halved on `tooBig`). Per-OID errors mark only that point Bad. An agent that stops answering turns all its points Bad and is retried.

i> Not yet: SNMPv3, SET Output, walks and traps.
"#,
        related: &["point-quality"],
    },
    // ───────────────────────────── Service ─────────────────────────────
    Article {
        id: "ext-protocols",
        title: "WITS, WebSocket, Sparkplug B, DNP3, CAN, Generator — how they work",
        category: Category::Protocols,
        accent: None,
        tags: &["wits", "websocket", "sparkplug", "dnp3", "can", "panel", "seen on the wire", "connection", "bind"],
        summary: "The shared panel and hub inspector layout of the five newer protocols.",
        body: r#"
The five newer protocols share one layout, so once you know one you know them all.

## The protocol panel

- A coloured banner with the protocol, connection count and how many links are up.
- **Connections** — + Add, select to edit, × to delete. Each has an **Id** (used in point addresses), a **Label** and an **Enabled** switch.
- **Status** — state dot, last error, received / sent / error counters, and connected peers for listeners.
- **Seen on the wire** — everything the link has received (WITS items, JSON paths, Sparkplug metrics, DNP3 points, CAN frames) with its latest value. Use it to check a link before you bind anything.

## Hub Input and Output nodes

1. Set the node protocol, pick the **Connection** and (where it applies) the poll / send interval.
2. Click a chip under **SEEN ON THE WIRE** to bind it straight away, or fill the **ADD BIND** form.
3. Inputs create points automatically (`wits_N`, `ws_N`, `spb_N`, `dnp3_N`, `can_N`); Outputs pick a host point.
4. Click a bind row to edit it in place; × removes it. The right column shows the live value, the dot its quality.

> Inputs may carry **scale / offset** (`value = raw × scale + offset`). Outputs that write numbers divide it back out.

i> Each protocol is its own pack in the service (`libpb_wits`, `libpb_websocket`, `libpb_sparkplug`, `libpb_dnp3`, `libpb_can`, `libpb_gen`). A pack that is not installed does not appear in the Protocols dropdown.
"#,
        related: &["wits", "websocket", "sparkplug", "dnp3", "can-bus", "generator"],
    },
    Article {
        id: "wits",
        title: "WITS (oilfield / frac record streams)",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0xff, 0x8f, 0x5a)),
        tags: &["wits", "wits0", "level 0", "frac", "drilling", "data van", "oilfield", "record", "item", "0108", "0121"],
        summary: "Receive and send WITS level 0 blocks over TCP or serial.",
        body: r#"
WITS level 0 is the ASCII record stream used between rigs, frac vans and service companies (not WITSML).

```
&&
01081234.5      record 01 item 08 = bit depth
01213500        record 01 item 21 = standpipe pressure
!!
```

## Connection

| Field | Notes |
|---|---|
| **Transport** | **TCP client** (dial the sender), **TCP listen** (senders / data vans connect to you) or **Serial** |
| **Host / Port** | Listen address or remote host; WITS has no fixed port (5100 in the demos) |
| **Serial port / Baud / Parity** | RS-232 / RS-485 links |
| **Stale after (s)** | No block this long → every Input point goes **Bad** until data returns |
| **Null values** | Values that mean "absent" (default `-9999,-999.25`) → that point goes Bad |

## Binds

- **Input**: item code (`0108`) → `wits_N`, optional scale / offset. Codes are 4 digits: 2-digit record + 2-digit item.
- **Output**: host point → item code, with decimals. All Output binds go out as **one block per send interval** to the peer (a listener sends to every connected data van).

> Frac vans and pressure-pumping companies often agree their own item list (record 11, 12 …). Any 4-digit code works; record 01 names are shown for reference.
"#,
        related: &["ext-protocols", "las", "industrial-demos"],
    },
    Article {
        id: "websocket",
        title: "WebSocket (JSON client and server)",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0x6c, 0xd4, 0xff)),
        tags: &["websocket", "ws", "wss", "json", "dashboard", "browser", "web", "hmi", "push"],
        summary: "Read JSON messages from a ws:// feed, or serve live points to browsers.",
        body: r#"
## Modes

| Mode | Use |
|---|---|
| **Client** | Dial a `ws://` / `wss://` URL (optionally send a **subscribe message** after connecting). Received JSON messages feed Input binds |
| **Server** | Listen on host:port. Every browser / dashboard that connects gets Output messages; messages they send feed Input binds — a two-way web HMI |

## WebSocket Output — no connection needed

A WebSocket **Output** node is its own server: add it, set a **Port** on the node (a free one from 8765 is picked for you) and it listens. Its connection appears in the WebSocket panel as *from Output node* and goes away with the node.

| Send by | Use |
|---|---|
| **Serve (listen on a port)** | The default. Browsers and apps connect to this machine |
| **Connect to a remote URL** | When you can't listen — behind an outbound-only firewall, or pushing to a cloud / Node-RED ingest server |
| **Use a WebSocket panel connection** | Share one socket you set up in the panel, e.g. a client feed that also takes commands |

**Auto-bind wired points** (on for new nodes) sends every point on an arrow into the node, JSON key = point id. Remove one with × to leave it out; manual binds still work alongside. Turning it off keeps the current binds as ordinary ones. The same option is on MQTT, HTTP, Streams, LAS and Sparkplug B Outputs (topic / JSON path / field / curve / metric named after the point) and on WITS Outputs (next free item code in a record you choose, 99 by default), and every Output's point picker lists only points wired into that node unless **Show all project points** is on.

To read what browsers send back (a two-way HMI), add a WebSocket **Input** and pick the Output node's connection — both sides then use the same socket. Inputs always need a connection, made in the panel or by an Output node.

## Binds

- **Input**: JSON path (`data.temp`, `items[0].value`; empty = the whole message) → `ws_N`. A message without that path leaves the point as it was, so mixed message types on one socket are fine.
- **Output**: host point → JSON key (empty = the point id).

## Output format

- **One object, all keys** — `{"ts": 1700000000000, "pressure": 3412.5, "rate": 82.1}` each interval when something changed. A new viewer gets the current object straight away.
- **One message per change** — `{"point":"p1","key":"pressure","value":3412.5,"ts":…}`.

> A browser page needs only `new WebSocket("ws://host:8765/")` and `JSON.parse(e.data)` — the demo projects serve dashboards on 8765–8767.
"#,
        related: &["ext-protocols", "json-paths", "industrial-demos"],
    },
    Article {
        id: "sparkplug",
        title: "Sparkplug B",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0xf2, 0x72, 0xb6)),
        tags: &["sparkplug", "spb", "spbv1.0", "mqtt", "birth", "death", "edge node", "scada", "uns", "metrics", "alias"],
        summary: "Consume Sparkplug B edge nodes as a host, or publish points as an edge node.",
        body: r#"
Sparkplug B is MQTT with a defined topic namespace (`spBv1.0/<group>/<type>/<edge>[/<device>]`) and protobuf payloads.

## Roles

| Role | Inputs | Outputs |
|---|---|---|
| **Host** | Metrics from NBIRTH / NDATA / DBIRTH / DDATA of the group (optionally one edge node) | **NCMD / DCMD** commands to an edge node |
| **Edge node** | NCMD / DCMD commands sent to this node | **NBIRTH** on connect (with `bdSeq`), DBIRTH per device, then **NDATA / DDATA** on change; NDEATH is the MQTT will |

## Binds

- **Input**: device (empty = node metric) + metric name → `spb_N`. Aliases from BIRTH are resolved automatically.
- **Output**: (host: target edge node) device + metric, and the BIRTH datatype (Auto picks Boolean / Int64 / Double / String from the first value).

## Behaviour

- A **DEATH** certificate turns that node's (or device's) points **Bad**.
- An edge node answers `Node Control/Rebirth` by publishing its BIRTH again.
- Connections point at any MQTT broker — the hub's embedded broker works (see the Bottling line demo).

!> The datatype of a metric is fixed by its BIRTH. Change Output binds and the edge node re-births on the next restart of the link.
"#,
        related: &["ext-protocols", "mqtt", "mqtt-broker", "industrial-demos"],
    },
    Article {
        id: "dnp3",
        title: "DNP3 master",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0xb8, 0xe0, 0x5c)),
        tags: &["dnp3", "ieee 1815", "rtu", "outstation", "master", "class 0", "events", "crob", "scada", "utility", "water"],
        summary: "Poll RTUs / outstations over TCP and operate their controls.",
        body: r#"
## Connection

| Field | Notes |
|---|---|
| **Outstation host / port** | TCP, usually port 20000 |
| **Master / outstation address** | DNP3 link addresses (0–65519) |
| **Timeout** | Per request |

## Inputs

Each poll reads **class 0** (all static values) plus **class 1 / 2 / 3** events. Bind **type + index** → `dnp3_N`:

| Type | Objects |
|---|---|
| Analog input | g30 / g32 |
| Binary input | g1 / g2 |
| Counter | g20 / g22 |
| Binary output status | g10 / g11 |
| Analog output status | g40 / g42 |

A point reported with its **ONLINE** flag cleared is **Bad**; a dead link turns every point Bad and is retried. **DEVICE RESTART** is cleared automatically.

## Outputs (controls)

- **CROB** — LATCH ON when the point is true / non-zero, LATCH OFF otherwise (direct operate, g12v1).
- **Analog output** — g41v3 float (default) or g41v1 32-bit integer; scale / offset applied in reverse.

A control the outstation refuses shows its status code as the connection error.

i> Serial DNP3, secure authentication and unsolicited-only outstations are not in this build (unsolicited messages are confirmed and applied).
"#,
        related: &["ext-protocols", "industrial-demos"],
    },
    Article {
        id: "can-bus",
        title: "CAN bus (SocketCAN, J1939)",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0xff, 0xd1, 0x66)),
        tags: &["can", "socketcan", "j1939", "pgn", "dbc", "signal", "vcan", "engine", "frame", "canbus"],
        summary: "Decode signals from CAN frames and transmit points as frames (Linux).",
        body: r#"
## Connection

**Interface** — `can0`, `can1`, or a virtual `vcan0` for testing. **Stale after** — no matching frame this long turns the point Bad.

```
sudo ip link add dev vcan0 type vcan
sudo ip link set up vcan0
```

## Signals (DBC style)

| Field | Notes |
|---|---|
| **Frame id** | `0x123` (11-bit), `0x18FEF100` (29-bit), or `pgn:61444` to match a J1939 parameter group from any source address |
| **Start bit / Bits** | Intel: start is the LSB. Motorola: DBC numbering, start is the MSB |
| **Order** | Intel (little-endian) or Motorola (big-endian) |
| **Type** | Unsigned, signed (two's complement) or float32 |
| **Scale / Offset** | `value = raw × scale + offset` (J1939 engine speed: scale 0.125) |

## Outputs

Each frame id gets an 8-byte buffer; every Output bind writes its signal into it and the frames are sent every transmit interval once a value is known. Set **DLC** for shorter frames.

!> SocketCAN is Linux-only. On other platforms the panel reports it.
"#,
        related: &["ext-protocols", "industrial-demos"],
    },
    Article {
        id: "generator",
        title: "Generator (synthetic data)",
        category: Category::Protocols,
        accent: Some(Color32::from_rgb(0x7d, 0xe0, 0x9a)),
        tags: &["generator", "gen", "synthetic", "simulate", "random", "random walk", "noise", "sine", "formula", "trigger", "burst", "demo data", "test data"],
        summary: "Make up data — waveforms, random walks, process shapes, counters, formulas — on a regular, random or burst timer.",
        body: r#"
A **Generator** is a source with no device behind it. It publishes ordinary points, so you can wire them to a hub, an Aggregator, a Calculation or an Output exactly like Modbus or MQTT inputs. Use it to demo a project, test a dashboard, or exercise downstream logic before real devices exist.

> **One direction only.** A generator is an Input: you can draw arrows *out of* it, but nothing can be wired *into* it.

## Add signals

1. Add a hub **Input** node and set its protocol to **Generator** (a board is created for you).
2. Click **+ Add signal**. Pick a tab — **Real-world** presets (temperature, tank level, pump running, pressure, flow, energy …) or a family: **Waveform**, **Random**, **Process**, **Digital**, **Counter**, **Formula**. Each card draws its own shape.
3. Name it, adjust the settings (only the ones that type has are shown), choose *when* it generates, and watch the **preview** redraw. The preview runs the same code as the pack.
4. **Add signal**. The point appears (`gen_N`, or the name you give the point) and goes live at once.

## Signal types

| Family | Types |
|---|---|
| Waveform | Constant, Sine, Square, Triangle, Sawtooth, Ramp (hold / wrap / bounce) |
| Random | Noise, Uniform, Random walk, Drift (a walk pulled back to a mean) |
| Process | Wandering setpoint (smooth chase of a random target), Sequence (your list of values) |
| Digital | On / off cycle, Toggle, Random on / off |
| Counter | Counter (step, optional roll-over), Totaliser (rate × time) |
| Formula | Your own expression |

## When it generates

| Trigger | Meaning |
|---|---|
| **Regular interval** | A value every N seconds |
| **Random timer** | The next value comes after a random wait between a minimum and a maximum — irregular reporters |
| **Bursts** | A quick run of values, then a random quiet spell — batch dumps and event storms |

Waveforms and totalisers advance by the *elapsed* time, so irregular triggers still give correct shapes.

## Formulas

`t` (seconds), `dt` (since the last value), `prev` (its last value), `n` (count), the **name of any other signal on the board**, and `+ - * / % ^`, comparisons, `&&`, `||`, `cond ? a : b`, `sin cos tan abs sqrt pow exp ln min max clamp round floor ceil if(c,a,b)`, plus random numbers: `rand()`, `rand(lo, hi)`, `randn()` (gaussian), `noise(x)`.

```
prev + randn() * 0.5                      a random walk
pump_run * (7.5 + 0.6 * sin(t / 9))       power only while another signal is true
50 + 10 * sin(t / 20) + noise(0.3)        a wave with jitter
```

The same language works in a **Calculation** node's *Expression / formula* (there `x` is the edge value).

## Board settings

- **Random seed** — `0` gives different data every run; any other number repeats the same data (each signal keeps its own stream).
- **Time scale** — model seconds per real second. `60` makes a one-minute waveform pass every second, handy for a "day" in a minute.

!> Changing a signal restarts its board: counters and random walks start again from their start values.

i> Try `generator-plant.json` in the projects folder: six generator nodes → an MQTT hub with every signal type and trigger.
"#,
        related: &["ext-protocols", "mqtt", "industrial-demos"],
    },
    Article {
        id: "running-service",
        title: "Running the service",
        category: Category::Service,
        accent: None,
        tags: &["service", "pb-service", "start", "dev.sh", "environment", "PB_MODULES", "PB_PROJECT", "PB_IPC_PORT", "ipc"],
        summary: "Start pb-service, the environment variables, and the dev script.",
        body: r#"
## Start it

```
cargo build -p pb-service -p pb-mqtt -p pb-modbus ...
mkdir -p modules && cp target/debug/libpb_*.so modules/
PB_MODULES=$PWD/modules cargo run -p pb-service
```

The service stays in the foreground and prints `IPC listening on 127.0.0.1:<port>`. Start the UI in a second terminal.

> **Shortcut:** `scripts/dev.sh` stops any running service / UI, rebuilds, refreshes `modules/`, starts the service and then the UI. Options: `--release`, `--project FILE`, `--no-ui`.

## Environment

| Variable | Meaning |
|---|---|
| `PB_MODULES` | Folder to load protocol packs from (unset = no packs) |
| `PB_PROJECT` | Project file to open |
| `PB_IPC_PORT` | Fixed localhost IPC port |

## How the UI finds it

The service writes `~/.config/protocolbench/ipc.json` with its port and a token. The UI reads it and talks length-prefixed JSON over localhost.
"#,
        related: &["protocol-packs", "status-bar", "service-and-ui"],
    },
    Article {
        id: "protocol-packs",
        title: "Protocol packs",
        category: Category::Service,
        accent: None,
        tags: &["packs", "modules", "plugins", "pack not loaded", "dll", "so", "abi"],
        summary: "Each protocol is a pack the service loads from the modules folder.",
        body: r#"
Each protocol is a **pack** — a shared library the service loads at start:

| Pack | File (Linux) |
|---|---|
| MQTT | `libpb_mqtt.so` |
| OPC UA | `libpb_opcua.so` |
| Streams | `libpb_streams.so` |
| HTTP | `libpb_http.so` |
| Modbus | `libpb_modbus.so` |
| File follow | `libpb_files.so` |
| SNMP | `libpb_snmp.so` |
| LAS | `libpb_las.so` |

On Windows the files are `pb_*.dll`.

## "Pack not loaded"

A protocol missing from the **Protocols** dropdown (or a panel showing *pack not loaded*) means the service did not find that library. Copy it into the service's `modules/` folder and restart the service (or run `scripts/dev.sh`).
"#,
        related: &["running-service", "troubleshooting"],
    },
    Article {
        id: "mcp",
        title: "MCP server (agents)",
        category: Category::Service,
        accent: None,
        tags: &["mcp", "agent", "ai", "claude", "model context protocol", "automation", "api", "tools"],
        summary: "Let an AI agent inspect and set up the service through MCP tools.",
        body: r#"
The service hosts an **MCP server** (Model Context Protocol, Streamable HTTP) on **127.0.0.1** only. Its URL is written to `mcp.json` in the config dir (`~/.config/protocolbench/`); requests need `Authorization: Bearer <ipc.token>` — the same token file the UI uses.

```
PB_MCP_PORT=7741 pb-service      # fixed port (default: any free port)
PB_MCP=off pb-service            # do not start it
```

## Tools

| Tool | Does |
|---|---|
| `get_status`, `list_protocols` | Project, packs, running hubs, per-protocol state and errors |
| `list_points`, `read_point` | Live values, quality and source addresses |
| `write_point` | Sends a value to every Output bound to that point |
| `list_hubs` | Hubs with their Input / Output nodes and bind counts |
| `get_project`, `set_project_section` | Read the project or replace one section (applied and saved) |
| `add_connection` | Add or replace a connection in modbus / wits / websocket / sparkplug / dnp3 / can |
| `open_project`, `stop_all_hubs` | Switch projects, stop everything |

## Connecting an agent

Add an HTTP MCP server with the URL from `mcp.json` and the bearer token header. For Claude Code:

```
claude mcp add --transport http protocol-bench http://127.0.0.1:7741/mcp --header "Authorization: Bearer $(cat ~/.config/protocolbench/ipc.token)"
```

!> Edits made through MCP are saved to the project straight away. An open UI keeps its own copy until the project is reopened — close the UI or reopen the project after agent changes.
"#,
        related: &["running-service", "security", "project-file"],
    },
    Article {
        id: "troubleshooting",
        title: "Troubleshooting",
        category: Category::Service,
        accent: Some(Proto::COPPER),
        tags: &["problems", "errors", "not working", "bad quality", "disconnected", "debug", "fix", "faq"],
        summary: "Common problems and where to look first.",
        body: r#"
## The status bar says *Service disconnected*

- Is the service running? `pgrep -a pb-service`
- Only one service should run — the UI attaches to whichever wrote `ipc.json` last.

## A panel says *pack not loaded*

Copy the pack library into `modules/` and restart the service. See **Protocol packs**.

## Points stay *Uncertain*

- The Input has binds but no data yet — check the connection is enabled and connected.
- For MQTT: is the Input's topic actually being published? Check the topic tree.

## Points turn *Bad*

The link died, timed out, or the device answered with an error. The pack keeps retrying. Check host, port, unit id, credentials and cables.

## A Modbus float reads as nonsense

Try byte order `cdab` (word swap). See **Modbus areas, types and byte order**.

## Nothing leaves on an Output

- Is there a **wire** into it, or binds added by hand? Check **Mappings**.
- Is the Output's connection connected?

## File follow / LAS shows no columns

The path is read on the **service** machine. The file must exist and have a header or a first row.

## A remote client cannot reach a hub

- Is the hub **started**? (Top-bar strip, Project panel.)
- Listening on `127.0.0.1` only accepts local clients — bind `0.0.0.0` for the LAN.
- Check the firewall for the port.
"#,
        related: &["status-bar", "protocol-packs", "point-quality", "traffic"],
    },
    Article {
        id: "security",
        title: "Security notes",
        category: Category::Service,
        accent: None,
        tags: &["security", "passwords", "tokens", "anonymous", "lan", "certificates", "keyring"],
        summary: "What is stored in plain text and which settings expose the LAN.",
        body: r#"
!> **Credentials are stored in the project JSON** — MQTT passwords, HTTP tokens, OPC UA passwords. There is no keyring yet. Keep project files private.

## Things that expose your LAN

- An **OPC UA server** with anonymous login bound to a non-loopback address.
- An **embedded MQTT broker** listening on `0.0.0.0`.
- A **Modbus server** — Modbus has no authentication at all; any master on the network can write mapped registers.

## Safer defaults

- Bind to `127.0.0.1` unless remote clients need access.
- Prefer **TLS / WSS** for MQTT across untrusted networks.
- Offer the **Sign&Encrypt** endpoint on OPC UA and turn off auto-trust once clients are trusted.
- The UI ↔ service link is localhost only and uses a token from the config folder.
"#,
        related: &["opcua-server", "mqtt-broker", "modbus-server", "project-file"],
    },
    // ───────────────────────────── Reference ─────────────────────────────
    Article {
        id: "json-paths",
        title: "JSON paths",
        category: Category::Reference,
        accent: None,
        tags: &["json", "path", "dotted", "array", "index", "mqtt", "http", "file"],
        summary: "One path syntax shared by MQTT, HTTP and File follow.",
        body: r#"
MQTT, HTTP and File follow (JSON lines) share one path syntax:

| Path | Selects |
|---|---|
| `temp` | Top-level field `temp` |
| `data.temp` | Nested field |
| `items[0]` | First array element |
| `items[2].value` | Field inside an array element |
| `$.data.temp` | Same as `data.temp` (leading `$` optional) |

Example payload:

```
{"data": {"temp": 21.5, "items": [{"value": 3}]}}
```

`data.temp` → `21.5` · `data.items[0].value` → `3`
"#,
        related: &["mqtt", "http", "file-follow"],
    },
    Article {
        id: "roadmap",
        title: "Not built yet",
        category: Category::Reference,
        accent: None,
        tags: &["roadmap", "planned", "future", "mcp", "limits", "snmpv3", "witsml", "serial dnp3"],
        summary: "Protocols and features that are planned but not in this build.",
        body: r#"
## Protocols

- SNMPv3, SNMP SET Output, walks and traps
- DNP3 over serial, DNP3 secure authentication, an outstation (server) role
- Sparkplug B DataSets / Templates, primary host STATE
- CAN FD, DBC file import
- WITSML (separate from WITS level 0, which is built)

## Product

- Keyring storage for credentials.
- License signing, Windows installer.

## Later

SQL, cloud connectors, Kafka, Influx line protocol, vendor PLC drivers (S7, EtherNet/IP, BACnet), historians, redundancy.
"#,
        related: &["welcome"],
    },
    Article {
        id: "glossary",
        title: "Glossary",
        category: Category::Reference,
        accent: None,
        tags: &["glossary", "terms", "definitions", "words"],
        summary: "Short definitions of the words used across Protocol Bench.",
        body: r#"
| Term | Meaning |
|---|---|
| **Bind** | A rule on a node: this field ↔ this point |
| **Connection** | A link to a device, broker, endpoint, file or port, set up in a protocol panel |
| **Emit** | A Streams Output's outgoing text line |
| **Feed / wire** | The data path between nodes |
| **Hub** | A routing point with one server protocol |
| **Origin hub** | The hub whose Input created a point |
| **Pack** | A protocol library the service loads |
| **Point** | A shared value with id, value, quality, source, time |
| **Quality** | Good, Uncertain or Bad |
| **Register map** | The list of Modbus registers on an Input, Output or Server connection |
| **Seen on the wire** | What a link has received, offered as one-click binds |
| **WITS item** | A 4-digit record + item code in a WITS level 0 block (`0108`) |
| **Sparkplug metric** | A named value in a Sparkplug B BIRTH / DATA payload |
| **CROB** | DNP3 control relay output block — a binary control |
| **PGN** | J1939 parameter group number; selects a CAN frame from any source |
| **Catalog** | What an OPC UA server hub publishes |
| **Wide subscribe** | MQTT filters for traffic preview only |
"#,
        related: &["concepts"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_and_related_resolve() {
        let mut seen = std::collections::HashSet::new();
        for a in ARTICLES {
            assert!(seen.insert(a.id), "duplicate help id {}", a.id);
        }
        for a in ARTICLES {
            for r in a.related {
                assert!(find(r).is_some(), "{} relates to missing {}", a.id, r);
            }
        }
    }

    #[test]
    fn every_category_has_articles() {
        for c in Category::ALL {
            assert!(ARTICLES.iter().any(|a| a.category == c), "{:?} empty", c);
        }
    }
}

# Protocol Bench

Protocol Bench is a native Rust bench for moving values between industrial and oilfield links. One process holds the points. A value that arrives on one protocol can leave on another. The screen and the converter live in the same project.

This build is the shell. Live protocol I/O comes later, on the engine threads. The window is egui (`eframe`), not a web UI.

## Shell

The interaction model follows Modbus Bench, with a Protocol Bench look:

- Docked panels. Drag a title to move one. Drop on the top of a panel, or on its tab strip, to add a tab. Several panels share one area. Left, right, and bottom edges still split. The center still swaps.
- A top bar: File, View, Help, theme chip, settings.
- A side rail of panel icons.
- Themes. Panel diagrams and the hub canvas keep the scope blue, mixed about halfway with the current theme background.
- Help → About: Agile Automation logo, centered “Protocol Bench” wordmark, version.

Panels in the shell today: Project, Hubs, Points, Mappings, Traffic, Streams, Modbus, MQTT, OPC UA, File follow.

## Engines

Each protocol engine owns its work on a thread the UI does not block. `pb-engine` is that boundary: the UI takes a snapshot and submits commands. The five slots that exist now (Streams, Modbus, MQTT, OPC UA, File follow) are idle. They do not open a serial port, a socket, or a file.

## Hubs

A hub is a conversion point. Sources come in. Outputs leave. A diagram shows what converts to what.

- More than one hub, so the bench is not forced onto a single protocol.
- A hub can be split across the protocols of its links, or locked to one protocol.
- The diagram is something you arrange: drag a node, click it for details, double-click or use “Open … panel” to show the matching protocol panel when the node is one protocol.
- Right-click a node or the canvas to add a source or an output.
- Middle-drag, or Space and left-drag, pans the canvas. Scrollbars remain.

Sample hubs ship so the diagram is not empty: a split Plant hub and a single-protocol Edge hub, with an MQTT link from one into the other. That is configuration framing, not a live bridge.

## What runs today, and what does not

Today: the window, layout, themes, the hub diagram, and the engine thread boundary.

Not yet: opening a port, speaking a protocol, parsing a file, or writing a point. No file requester in this build. When one is added, it must not block the UI thread. A blocking dialog, or a vsync GL swap while the window sits on another Hyprland workspace, is what makes the compositor call the app “not responding.”

## Protocols

Already named in the shell:

- Raw serial (Streams)
- Modbus
- MQTT
- OPC UA
- File follow

Also in the product, not all drawn as their own panels yet:

1. Serial ASCII and custom line protocols
2. HTTP REST / JSON gateways
3. WebSocket
4. Sparkplug B
5. DNP3
6. SNMP
7. CAN / SocketCAN
8. LAS
9. WITS

LAS and WITS are the oilfield and frac data plane. WITS here means the WITS record stream, not WITSML. WITSML is a later, separate decision.

Notes on why each is on the list, and which crates are worth a spike, are in [protocol-options.md](protocol-options.md).

## Polish still in front of us

- Live link state, quality, and the last error on each endpoint.
- A saved project that restores endpoints and mappings, beyond the window layout that already persists.
- File follow that remembers a byte offset across a restart and a rotated log.
- The rest of the protocol list above, each on its own engine thread.
- Panel art that keeps following the theme as new diagrams are added.

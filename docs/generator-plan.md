# Generator source — plan

Status: **built (2026-11-27).** What shipped, and where it differs from the plan below:

| Planned | Built |
| --- | --- |
| `pb-expr` shared evaluator | `pb-expr` — also now runs **Calculation-node formulas** (the engine never evaluated them before; only the UI preview did arithmetic) |
| `pb-gen` pack with models | `pb-gen` (clock + publishing) over **`pb-signal`** (pure models, triggers, previews). The split exists so the UI can draw live previews from the same code; a pack crate cannot be linked into the UI. |
| Triggers: interval, random timer, burst, on demand | interval, random timer, burst. *On demand* (a fire-once button) was left out — it needs a command channel into the worker. |
| Dedicated panel | Dedicated hub **Input inspector** (`gen_ui.rs`: category tabs, model cards that draw their own shape, 16 real-world presets, live preview) plus the schema-driven panel for the board (seed, time scale), live signals and a signal-type reference |
| Boards as JSON templates | **Real-world presets** (per signal) instead. Coupled multi-signal templates (tank + pump) are achieved with formulas that read other signals; whole-board templates are future work. |
| Time scale | Built (board-level, scales model time `t` / `dt`; triggers stay wall-clock) |
| Seed | Per board; each signal gets its own stream derived from the seed and its name |
| Write-back, fault simulation, CSV replay | Not built (phase 2) |

Demo: `protocol-bench-projects/generator-plant.json` (six generator nodes → MQTT hub). Tests: unit tests in `pb-expr`, `pb-signal`, `pb-gen`, `pb-engine`, `pb-ui`, `pb-service`; end to end in `scripts/demo-smoke.sh generator` (points good, MQTT topics published, data moving via `pb-sim mqtt-check`).

---

Original plan (2026-11-27):

A **Generator** is a protocol-pack-style *source* that invents data instead of reading a device. It publishes ordinary points, so everything downstream (hub, Aggregator, Calculation, Output, Mappings, MQTT/OPC UA/Modbus egress) works unchanged.

## Wiring rules (hub diagram)

* A Generator is an **Input-role node** (`Role::Source` in `pb-ui/src/hubs.rs`): a *feeder* only.
* You can draw wires **out of** it to a Hub, Aggregator, Calculation or Output, like any other Input.
* **Nothing can be wired into it.** It must never count as a sink: `is_sink()` stays false for it, and `connect_wire_nodes`, `connect_feed` and `rewire_feed` refuse it as a target. Add a unit test next to the existing wire tests (`deleting_output_keeps_connected_calc_and_drops_feeds`, `calc_wires_into_hub_when_dragged_to_it`) that tries a drag in both directions.
* No egress: `write_point` is a no-op in v1 (the pack is read-only, like the default in `pack-development.md`).
* Setpoint write-back (a point written from outside steers a signal) is **phase 2** and would be an explicit per-signal opt-in, not a wire into the node.

## Triggers: when a signal generates

Every signal has a **trigger**. Several signals can share one (a board's signals usually tick together so related values stay coherent).

| Trigger | Behaviour |
| --- | --- |
| **Interval** | Fixed period (100 ms … hours). Optional start delay and phase offset. |
| **Random timer** | Next fire = uniform random in `[min, max]` (optionally exponential with a mean, i.e. Poisson arrivals). Models irregular reporters, event-driven telemetry, "report by exception". |
| **Impulse / burst** | A short train of N samples at a rate, then idle for a period (fixed or random). Models batch dumps and alarms. |
| **On demand** | A manual "fire once" button in the panel (and later a trigger point). Good for demos and for testing downstream wiring. |

Rules:

* Time-based models (sine, ramp, random walk, formula) advance by **elapsed model time since their last sample**, not by a nominal period, so irregular triggers still produce correct waveforms. `dt` in formulas is that elapsed time.
* Each fire publishes through `host.upsert_f64` / `upsert_i64` / `upsert_bool`, with `PB_QUALITY_GOOD`. A generator never publishes bad quality unless a signal is deliberately configured as "fault" (phase 2: simulate a sensor dropout so quality handling can be demoed).
* Stop publishes nothing and keeps the last value; Start resumes from the held state (a random walk continues from where it was).
* Optional **time scale** per board (1× default; 60× compresses a minute into a second). A scale applies to model time, not to trigger wall-clock, and is a single field, so it is cheap to leave out of v1 if it complicates the first cut.

## Components

### 1. `pb-expr` (plain rlib)

The shared expression evaluator. Replaces `eval_simple_arith` in `pb-ui/src/calc_panel.rs` and gives the engine calc node ([pb-engine/src/calc.rs](../pb-engine/src/calc.rs)) a real `formula` kind.

* Syntax: numbers, `+ - * / %`, comparison and `? :`, parentheses, unary minus.
* Variables: `x`/`A`/`B` (calc inputs), `t` (model seconds), `dt`, `prev` (this signal's last value), plus other signals by name inside a generator board.
* Functions: `sin cos tan abs sqrt pow exp ln min max clamp floor ceil round sign`, `rand()` (uniform 0‥1), `randn()` (gaussian), `noise(amp)`.
* **Compile once** to a small AST/bytecode, evaluate per tick; parse errors are reported with the column so the UI can show them next to the field.
* Seeded RNG (`xorshift`/`pcg`, no dependency) held in the evaluator. A board with a seed is repeatable; without one it seeds from the clock.
* Pure and `Send`, no I/O, so it is unit-testable and usable from the UI preview and the engine.

This also answers the original question: calc nodes get `prev + randn()*0.2`.

### 2. `pb-gen` (cdylib pack)

Follows [pack-development.md](pack-development.md): `src/lib.rs` (models, scheduler), `src/pack.rs` (`PackState`, `impl pb_abi::Pack`, `export_pack!`).

* One worker thread per board, a min-heap of next-fire times, short sleeps via `pb_abi::worker::sleep_stop`.
* `configure(json)` is idempotent: a board whose config key is unchanged keeps running and keeps its state (random walk position, counter).
* Signal models (v1): constant, ramp, sawtooth, sine, square, pulse, gaussian noise, random walk (start, step, min/max, optional mean reversion), counter, step schedule, **formula**.
* Point address: `gen/<board>::<signal>`. Add `gen` to the table in `pb-points/src/addr.rs`; the UI and service build it with the same function, never by hand.
* `detail_json()` returns per-signal last value, last fire time and any formula error, for the panel's live column.
* No `unsafe` of its own; the macro handles the ABI.

Fit with the ext-pack path: a board is the "connection", but it has no host/port, so the `ExtSpec` flow needs either a small allowance for "connection without address" or `pb-gen` gets a dedicated panel instead of the generic ext panel. Decide while doing the UI step.

### 3. Boards (templates)

Data, not code: a JSON file per board listing signals, triggers and formulas. Shipped in-tree for the demos, and user-added boards are just more JSON files.

Candidates (patterned on `pb-sim`'s water / bottling / frac and the Modbus-bench-style plants):

| Board | Signals / behaviour |
| --- | --- |
| Tank + pump | level fills with inflow; pump hysteresis (start high, stop low); flow, pump run, level alarm |
| Temperature loop | setpoint, PV approaching it with lag + noise, output % |
| Compressor | pressure cycling between load/unload thresholds, motor amps, run hours counter |
| 3-phase power | V L-N ×3, I ×3, PF, kW, kWh counter |
| Drilling | depth, ROP, WOB, RPM, torque, pump pressure (pairs with the WITS pack) |
| Diurnal flow / weather | sinusoidal day curve + noise + random-walk drift |
| Random walk (basic) | one signal, the minimal case |

Coupled boards (tank + pump) need signals to read each other: formulas reference other signals **in the same board**, evaluated in dependency order within a tick. Reference cycles are rejected at configure time with a clear error. A signal's reference reads that signal's most recent published value, so signals on different triggers still work (they see the latest value, not a synchronized one).

### 4. UI (`pb-ui`)

* New `HubProto::Generator` (or a dedicated Input node kind) and `DockPane` entry; persistence as a `generator` section of `project.json`.
* Generator panel: board picker ("Add board…"), signal table (name, type, model, parameters, trigger, live value, sparkline), formula field with inline error, Start/Stop, "fire once".
* Hub inspector for the Generator node: pick which board/signals this Input exposes; each exposed signal becomes a bind row (same bind mechanics as other Inputs), so allocation of short point ids (`gen_1`), Points and Mappings rows come for free.
* Help article in `help_catalog.rs`.

### 5. Service

`build_gen_configure` in [pb-service/src/runtime.rs](../pb-service/src/runtime.rs) turns boards into the pack's `{boards, signals}` JSON, with a unit test pinning the JSON (as the doc requires for any new config).

## Build order

1. **`pb-expr`** with tests, wired into the calc node and calc preview. Useful on its own and unblocks everything else.
2. **`pb-gen` core**: scheduler + triggers + basic models + point publishing. Test with a fake host (like `pb-snmp`'s pack tests), then a real isolated service (netns/private XDG per the isolated-testing rule, never Mike's running service).
3. **Wiring rules + UI**: Generator node as a feeder-only Input; panel; persistence; the "cannot wire in" tests.
4. **Boards**: JSON templates + picker, starting with random walk, tank + pump, temperature loop.
5. **Phase 2**: setpoint write-back, simulated faults/bad quality, time scale if cut from v1, CSV replay.

All new code in Rust (including any demo/sim helpers), per the project rule.

## Testing

* `pb-expr`: precedence, functions, error columns, seeded repeatability, `prev`/`dt` semantics.
* `pb-gen`: trigger timing (interval jitter bounds, random-timer range, burst count), state held across reconfigure, cycle detection, quality always good, Stop/Start continuity.
* UI/hub: generator is never a sink; wires out to Hub/Agg/Calc/Output are accepted; loop and Hub→Hub rules unchanged.
* End to end: a generator board feeding a Modbus server hub, read back by a client, in an isolated netns.

## Open questions

1. **"Impulse" trigger**: I read it as a short burst of samples. If you meant something else (e.g. a single one-off pulse, or an on-change trigger), say so and I'll adjust the table.
2. **Modbus Binge reference**: which of its boards or features should be mirrored? A list or screenshots would be enough since I can't see that tool.
3. **Time scale** (compress a day into a minute): v1 or later?
4. **Seed**: per-board seed, or one project-level seed?
5. **Ext-panel vs dedicated panel**: no address/port makes the generic ext path an awkward fit; I lean toward a dedicated panel.

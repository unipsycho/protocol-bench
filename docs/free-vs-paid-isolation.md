---
cursor:
  subagentId: "bc-43569496-cb29-512d-8f4f-c1b35187faaf"
---

# Protocol Bench — free vs paid isolation (options)

Date: 2026-11-12. Status: **options only — no decision**. Complements [prd.md](prd.md). TopicBench OSS / Pro is cited only as a comparable pattern Mike already ships; this design is for Protocol Bench.

## Why this note

The shell already names Streams, Modbus, MQTT, OPC UA, and File follow, with a longer product list (ASCII lines, REST, WebSocket, Sparkplug B, DNP3, SNMP, CAN, LAS, WITS). Competitors often gate by driver, tag count, or northbound feature. Protocol Bench still has an open license decision (PRD § Open decisions #9). The hard product question is how free and paid **protocol modules** isolate — in the binary, on disk, and in Linux packaging.

## Protocol shortlist (for placement talk)

| Tier (suggested for discussion only) | Protocols |
| --- | --- |
| Core / likely free | Raw serial (Streams), File follow (CSV / JSON lines), maybe one “obvious” I/O path so the free app is useful alone |
| High-value industrial | Modbus, MQTT, OPC UA |
| Oilfield / niche | LAS, WITS |
| Later / specialty | Serial ASCII, REST, WebSocket, Sparkplug B, DNP3, SNMP, CAN / SocketCAN |

Any option below must say where each row lands. The table is not a recommendation.

## Comparable pattern (TopicBench only)

TopicBench today: **public OSS binary** + **private `topicbench-pro` repo** + **signed machine-bound license** unlocking all Pro features, with paid engines moving toward a **localhost sidecar**. Free app stays unrestricted for base MQTT explorer features. Linux ships a bare glibc binary (in-app self-replace); `.deb` / AppImage are not the canonical release path.

Protocol Bench differs: value is **which protocols speak**, not mostly UI extras. Isolation choices that fit MQTT explorer Pro features may fit protocol engines poorly (or vice versa).

---

## A. Separate free and paid builds

Different binaries / products (closest to “TopicBench OSS vs topicbench-pro” as two shippable trees). Example: `protocol-bench` (free) and `protocol-bench-pro` (paid), or free vs paid crates built with different feature sets into two artifacts.

### How it works

- Free tree compiles only free protocol engines; paid tree compiles the rest (or everything).
- Distribution: two GitHub products, two download pages, or free public + paid private zip (TopicBench-like).
- Shared UI/shell via workspace crates; paid engines stay out of the free Cargo graph where possible.

### Pros

- Clearest IP boundary: paid protocol code never ships in the free binary.
- Matches buyer mental model of “buy Pro” as a different download.
- License enforcement is mostly “you have the Pro package,” not runtime gates inside free.
- Easier to keep free OSS (if open-sourced) without copyleft / MPL concerns from paid stacks bleeding into the free artifact (e.g. OPC UA crate license decisions stay in Pro).

### Cons

- Two release trains, two CI matrices, two Windows + Linux artifacts every version bump.
- Bugfixes in the shell must land in both trees (or a shared core with careful versioning).
- Users cannot “unlock” without installing another binary; migration of saved projects between editions needs a compatibility story.
- Panel surface area: free build either hides paid panels or shows locked stubs — both need UX design.

### Linux packaging (incl. Debian)

| Channel | Free | Paid |
| --- | --- | --- |
| Bare binary (TopicBench-style) | `protocol-bench-linux-x64` | `protocol-bench-pro-linux-x64` |
| Debian / `.deb` | Package `protocol-bench`; Conflicts/Replaces policy vs Pro must be explicit | Package `protocol-bench-pro` — same `/usr/bin` name collision risk; prefer distinct binary names |
| AUR / pacman | `-bin` wrapping the free release asset | Private or customers-only PKGBUILD, or skip AUR for Pro |
| Updates | In-app GitHub Releases self-replace for bare binary; apt for `.deb` | Separate update channel (private release host, customers portal) — cannot share free’s public Releases blindly |

Debian note: two packages providing the same `/usr/bin/protocol-bench` will fight. Use `protocol-bench` vs `protocol-bench-pro` binary names, or a single package name that switches via alternative — alternatives are awkward for GUI desktop entries.

### Updates

- Free: public Releases + optional in-app updater (TopicBench pattern).
- Paid: private feed or portal; updater must authenticate or ship only to licensed customers.
- Version skew: document “Pro N requires free shell N” or ship Pro as a full standalone so skew is irrelevant.

### Protocol shortlist fit

Strong when the paid set is large or license-sensitive (OPC UA, DNP3, LAS/WITS). Weak if Mike wants “try Modbus free, unlock OPC UA” without a second install.

### Open questions for Mike

1. Is Pro a **full standalone** or an **add-on** beside free?
2. Same project file format across editions?
3. Will free be open source (public GitHub) while Pro stays private?
4. Distinct binary / `.desktop` / AppStream IDs from day one?

---

## B. Single app + paid protocol modules (plugins / dylibs)

One shell binary; paid protocols load as dynamic libraries (or separate plugin files) beside the app.

### How it works

- `pb-engine` grows a stable plugin ABI (or `libloading` of `cdylib` crates): `pb_modbus.so`, `pb_opcua.so`, …
- Free install ships shell + free modules; paid modules are separate downloads unlocked by purchase.
- Panels appear when a module is present and licensed.

### Pros

- One UI binary to update for shell bugs.
- Sell protocols à la carte (Kepware / Neuron driver-suite feel) without rebuilding the shell.
- Paid code can stay off the free disk image until purchased.
- Matches industrial buyers used to “driver packs.”

### Cons

- **ABI stability** is hard in Rust; plugins often force a C ABI, versioned structs, and careful `pb-engine` trait object design — real engineering cost before first paid module ships.
- Security: loading customer-adjacent `.so` files means signature checks, search-path rules, and “do not load from world-writable dirs.”
- Linux packaging: plugins must live in a known dir (`/usr/lib/protocol-bench/`, next to the AppImage, or `~/.local/share/…`) with upgrades that do not orphan modules.
- Windows: DLL hell / side-by-side versions; code signing story for each module.
- Debugging “works on Mike’s machine” when plugin and shell minor versions diverge.

### Licensing

- Module file presence is weak DRM; pair with a signed license that lists allowed module IDs (or a license per module).
- Stolen `.so` without license: decide whether the module refuses to start (preferred) or the shell refuses to load it.

### Linux packaging (incl. Debian)

| Piece | Suggestion |
| --- | --- |
| Shell package | `protocol-bench` → binary + free modules |
| Paid modules | `protocol-bench-mod-opcua`, `…-modbus`, … each installing into `/usr/lib/protocol-bench/modules/` |
| Dependencies | Module packages `Depends: protocol-bench (>= X.Y)` |
| Bare portable | Zip layout: `protocol-bench` + `modules/*.so` next to the binary (updater replaces shell and free modules; paid modules update separately) |

Debian shines here if Mike wants apt-installable protocol packs. Bare-binary-only (TopicBench style) makes module path and updater rules more home-grown.

### Updates

- Shell updates must not break module ABI without a major version bump.
- Module updates can ship without a shell rebuild — the main operational win.
- In-app updater needs a manifest: shell version + module list + signatures.

### Protocol shortlist fit

Best when protocols are sold **individually** or in small packs (e.g. Oilfield Pack = LAS+WITS). Overkill if Pro is always “everything.”

### Open questions for Mike

1. À la carte protocols, packs, or always-all-paid-modules?
2. Willing to invest in a versioned plugin ABI before engines are even live?
3. Must plugins work offline forever after purchase (typical for plant PCs)?
4. Sign modules with the same key story as TopicBench licenses, or separate?

---

## C. Single binary + license unlock

All protocol code ships in one binary; a signed license (or entitlement list) enables paid engines at runtime.

### How it works

- Free download includes Modbus / MQTT / OPC UA / … code paths.
- Without a license, paid engines stay idle (panels locked or hidden); with a valid license, they start.
- Closest to TopicBench’s **license file unlock**, but Protocol Bench would gate **engines**, not only UI extras. TopicBench also splits some Pro work into a sidecar — optional later here if isolation of process crash/IP matters.

### Pros

- Simplest packaging and updates: one Linux binary, one Windows artifact, one Debian package.
- Instant unlock after dropping `license.json` — no second install.
- Shared project files just work when the user upgrades.
- Lowest engineering overhead while engines are still idle stubs.

### Cons

- **Paid protocol code is on every free disk.** Determined users can patch gates; acceptable for TopicBench-style indie pricing, weaker if protocol IP or third-party license obligations demand separation.
- Copyleft / MPL (e.g. some OPC UA stacks): shipping in a free binary may force license obligations on the whole distribution even when the feature is gated off — **check before choosing C for OPC UA**.
- Marketing must explain “the free download contains Pro code that stays locked.”
- Harder to open-source the same tree without revealing paid engines (unless paid stays behind a private feature branch / private crate pulled only into release builds — which starts looking like A).

### Licensing

- Machine-bound signed license (TopicBench pattern) with either:
  - **All-or-nothing Pro** (one bit unlocks all paid protocols), or
  - **Entitlement list** (`modbus`, `opcua`, `oilfield`, …) if à la carte matters without plugins.

### Linux packaging (incl. Debian)

| Channel | Shape |
| --- | --- |
| Bare binary | One `protocol-bench-linux-x64`; license in `$XDG_CONFIG_HOME/protocol-bench/` |
| Debian | One package; license file in `/etc/protocol-bench/` or user config — document that root-owned installs still read per-user or system license paths |
| Updates | Single public or freemium Release stream; license remains valid across updates until expiry |

No second package. Simplest apt and AUR story.

### Updates

- Same as a normal single app. License file survives binary replace.
- Entitlement changes = re-issue license, not reinstall.

### Protocol shortlist fit

Strong default while the product is early: ship everything gated, decide free/paid lists in the license without rebuilds. Weak if free OSS + hidden paid source is a goal, or if crate licenses forbid the combined binary.

### Open questions for Mike

1. All-or-nothing Pro vs per-protocol entitlements in the license file?
2. Is shipping locked OPC UA / DNP3 code in the free binary acceptable legally and commercially?
3. Reuse TopicBench ed25519 license format (new product field) or a fresh scheme?
4. Trial: time-limited license, or unlimited free protocols only?

---

## D. Hybrid — free core protocols + paid modules or Pro for the rest

Free binary includes a useful core (e.g. Streams + File follow + maybe MQTT **or** Modbus). Everything else is either a Pro unlock (C-style), separate Pro build (A), or loadable modules (B).

### How it works

Pick a free floor that makes the app worth installing alone, then apply A, B, or C only to the paid remainder.

Example hybrids:

- **D+C:** free engines compiled in and always on; paid engines compiled in but license-gated.
- **D+A:** free public binary; Pro binary adds oilfield + OPC UA + … 
- **D+B:** free engines built-in; paid engines as signed `.so` packs.

### Pros

- Honest free tier (competitors often give Modbus+MQTT free-ish; Neuron’s free tier is tag-capped).
- Defers full plugin ABI or dual-repo cost until paid protocols exist.
- Lets Mike change the free/paid line later with less user anger if the free core stays stable.

### Cons

- Still must choose the paid isolation mechanism (A/B/C) — hybrid is a **product split**, not a packaging answer by itself.
- Boundary debates: is MQTT free (expectation for “edge”) or paid (high value)? Is Modbus free because Modbus Bench exists?
- Support matrix: “which protocols does free include in v0.x?” must be documented and kept.

### Linux packaging (incl. Debian)

Follow the chosen paid mechanism:

- **D+C:** one package (simplest).
- **D+A:** two packages / two binaries (see A).
- **D+B:** shell + optional module packages (see B).

Debian users benefit most if the free package alone is apt-complete for a real job (serial + file follow + one network protocol).

### Updates

Same as the underlying A/B/C choice. Document that free-core projects remain openable in Pro; Pro-only endpoints show a clear “needs Pro / module” when opened in free.

### Protocol shortlist fit

| Candidate free core | Rationale |
| --- | --- |
| Streams + File follow | Always useful; no rich protocol stack; low IP risk |
| + MQTT | Common northbound; matches “edge” demos |
| + Modbus | Overlaps Modbus Bench — may undercut or complement that product |
| Paid: OPC UA, DNP3, LAS, WITS, Sparkplug, … | Higher willingness to pay; some have heavier license/crate issues |

### Open questions for Mike

1. What is the **minimum free** set that still feels like Protocol Bench, not a hollow shell?
2. Does free MQTT or free Modbus hurt Modbus Bench or TopicBench positioning?
3. Oilfield (LAS/WITS): always Pro, or a separate “Oilfield Pack”?
4. Which paid isolation (A, B, or C) should hybrid sit on for v1?

---

## Cross-cutting: Linux packaging snapshot

| Option | Artifacts | Debian fit | Updater complexity |
| --- | --- | --- | --- |
| A Separate builds | 2 binaries | 2 packages; avoid name collision | 2 channels |
| B Plugins | 1 binary + N modules | Excellent (module packages) | Manifest + ABI |
| C License unlock | 1 binary | 1 package | Lowest |
| D Hybrid | Depends on A/B/C | Prefer free package useful alone | Depends |

TopicBench’s current preference (bare Linux binary + in-app replace; `.deb` not canonical) favors **C** or **D+C** operationally. Choosing **B** or Debian-first module packs is a conscious shift toward apt-style distribution.

## Cross-cutting: security

- **A:** paid code absent from free — strongest containment; trust the Pro download channel.
- **B:** treat modules like code execution — sign, verify path, pin ABI; refuse unsigned modules in release builds.
- **C:** gate is honor + obfuscation-thin; fine for indie Pro, weak against reverse engineering; still verify license signatures the way TopicBench does.
- All options: license files machine-bound if piracy on shared plant images matters; offline verify (no phone-home required for field PCs).

## Cross-cutting: how this interacts with `pb-engine`

Today engines are idle slots behind a trait boundary — good news. Isolation can be designed at the trait object registration layer:

- **A/C/D+C:** `register_engines()` compiles different sets or skips unlicensed ones.
- **B:** `register_engines()` scans a module directory and loads cdylibs that export a fixed C ABI into the same trait objects.

No packaging code is implied yet; only that the engine boundary should not assume “all five slots are always compiled and always runnable” if Mike picks A or B.

---

## Top tradeoffs (no winner)

1. **IP / crate license separation vs ship speed** — A/B keep paid code off free disks; C is fastest while engines are stubs.
2. **One update train vs à la carte** — C wins on ops; B wins on selling protocols like drivers; A wins on clean product split.
3. **Rust plugin ABI cost** — B’s real price; avoid unless à la carte modules are a firm product goal.
4. **Free tier honesty** — D forces an explicit free protocol floor; A/C can still ship a hollow free app if the free set is too thin.
5. **Debian / apt** — B maps naturally to module packages; C maps to one package; A needs careful naming.

## Suggested decision inputs for Mike (checklist)

- [ ] Open-source free edition? (If yes, prefer A or D+A for paid source containment, or private crates behind C release builds.)
- [ ] Sell all-or-nothing Pro, protocol packs, or single protocols?
- [ ] Accept paid code inside the free binary? (Legal + commercial)
- [ ] Canonical Linux install: bare binary (TopicBench-like), Debian, or both?
- [ ] Free protocol floor (Streams / File / MQTT / Modbus / …)?
- [ ] Relationship to Modbus Bench and TopicBench pricing stories?
- [ ] Reuse TopicBench license machinery (keys, machine id, portal) for Protocol Bench?

## Related paths

- Repo PRD: `/home/mike/Documents/AAI-Projects/protocol bench/docs/prd.md`
- Store PRD: [prd.md](prd.md)
- Landscape: [competitive-landscape.md](competitive-landscape.md)
- In-repo protocol notes: `/home/mike/Documents/AAI-Projects/protocol bench/docs/protocol-options.md`

# Performance — CPU and memory

Measured 2026-11-26 on the release build (Linux, x86-64). Method: each scenario runs in a private
network namespace (so ports never collide with a running service), the service loads all 13
packs, `pb-sim` plays the field devices, and the UI draws on Xvfb. After 8 s to settle,
`pb-sim perf <pid …> --secs 15` samples `/proc` once a second.

> **Caveat for UI numbers:** Xvfb has no GPU, so Mesa's software rasterizer (llvmpipe) draws
> every frame on the CPU. Per-thread accounting separates it: with the frac demo live, the UI's
> own thread (egui layout, panels, IPC) used **3.5 %** of a core and llvmpipe **63.5 %**. On a
> desktop with a GPU, expect roughly the first number.

## Service (`pb-service`)

| Scenario | CPU avg | RSS | Threads |
| --- | --- | --- | --- |
| Blank project, 13 packs loaded, nothing configured | 0.1 % | 15.8 MiB | 3 |
| Frac spread, devices absent (every link retrying) | 0.4 % | 19.9 MiB | 15 |
| Frac spread + `pb-sim frac`, hubs running | 0.5 % | 20.7 MiB | 18 |
| …with the UI attached (snapshot polling) | 0.8–0.9 % | 21.5 MiB | 19 |
| Bottling line + sim + UI | 0.7 % | 21.1 MiB | 14 |
| Water lift station + sim + UI | 0.5 % | 18.2 MiB | 7 |
| TCP loopback demo + UI | 0.4 % | 19.2 MiB | 11 |

The service is cheap in every case: under 1 % of one core and about 20 MiB. Retrying dead
links costs nothing measurable.

## UI (`protocol-bench`), software rendering

| Scenario | Before | After | Frames / s (after) |
| --- | --- | --- | --- |
| Default project, nothing live | 91 % | **13 %** | 1.0 |
| Frac spread, live values + running hubs | 147 % | **68 %** | 5.4 |

RSS stays at 116–121 MiB in every scenario (most of it is the GL stack and font atlas).

### What changed

- The window used to request a repaint every 150 ms forever, so an idle UI redrew ~7 times a
  second. Now it polls once a second after ~1.4 s without a change (enough for uptime clocks)
  and goes back to the fast poll on the first new value. Mouse and keyboard input still
  redraw at once (`next_poll_delay` in `pb-ui/src/app.rs`, unit-tested).
- The live poll is 200 ms (was 150 ms) and the hub status strip's pulse / timer repaint is
  250 ms (was 100 ms, which alone forced ~10 frames / s whenever a hub ran).

## Re-running

```bash
cargo build --release -p pb-service -p pb-ui -p pb-sim   # plus the packs
pb-sim perf <pid> [pid …] --secs 15 --label NAME           # any running process
PB_FPS=1 protocol-bench                                    # prints frames per second to stderr
```

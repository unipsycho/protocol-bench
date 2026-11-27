#!/usr/bin/env bash
# End-to-end demo smoke: pb-sim field devices -> pb-service with every pack -> points good.
# Runs each industrial demo project (frac spread, bottling line, lift station) in a private
# config dir, so a running UI never attaches to this throwaway service. All Rust (no python).
#
#   scripts/demo-smoke.sh            # all four
#   scripts/demo-smoke.sh frac       # one (frac | water | bottling | generator)
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PROJECTS="$ROOT/../protocol-bench-projects"
cargo build -q -p pb-mqtt -p pb-opcua -p pb-modbus -p pb-las -p pb-wits -p pb-websocket -p pb-sparkplug -p pb-dnp3 -p pb-can -p pb-gen -p pb-dashboard -p pb-service -p pb-sim || exit 1
WORK="$(mktemp -d)"
cleanup() { [ -n "${SVC:-}" ] && kill "$SVC" 2>/dev/null; [ -n "${SIM:-}" ] && kill "$SIM" 2>/dev/null; wait 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT
mkdir -p "$WORK/modules"
cp -f target/debug/libpb_*.so "$WORK/modules/"
export XDG_CONFIG_HOME="$WORK/cfg"
mkdir -p "$XDG_CONFIG_HOME/protocolbench"
FAILED=()

run_demo() {
  local name="$1" sim="$2" project="$3"; shift 3
  echo "=== $name ==="
  cp "$PROJECTS/$project" "$WORK/$project"
  rm -f "$XDG_CONFIG_HOME/protocolbench/ipc.json"
  # The generator demo makes its own data: no field simulator.
  if [ "$sim" != none ]; then target/debug/pb-sim "$sim" >"$WORK/sim-$sim.log" 2>&1 & SIM=$!; fi
  PB_MODULES="$WORK/modules" PB_PROJECT="$WORK/$project" PB_AUTO_RUN_HUBS=1 target/debug/pb-service >"$WORK/svc-$sim.log" 2>&1 & SVC=$!
  if target/debug/pb-sim check "$@" --timeout 45 && { [ -z "${MQTT_CHECK:-}" ] || target/debug/pb-sim mqtt-check $MQTT_CHECK; }; then
    echo "--- $name: ok"
  else
    echo "--- $name: FAILED"; tail -n 30 "$WORK/svc-$sim.log"; FAILED+=("$name")
  fi
  kill "$SVC" ${SIM:+"$SIM"} 2>/dev/null; wait "$SVC" ${SIM:+"$SIM"} 2>/dev/null; SVC=""; SIM=""
  sleep 0.5
}

want="${1:-all}"
if [[ $want == all || $want == frac ]]; then
  run_demo "Frac spread" frac frac-spread.json \
    p1_discharge_psi p4_rate p2_coolant_f blender_sand_ppa wh_treating_psi wh_stage wh_frac_valve_open wits_plug_depth_ft
fi
if [[ $want == all || $want == water ]]; then
  run_demo "Lift station" water water-lift-station.json \
    wet_well_ft flow_gpm pump1_starts high_level_alarm cl2_free_mgl cl2_ph
fi
if [[ $want == all || $want == bottling ]]; then
  # Sparkplug metrics need the hub's embedded broker (:1886), started by auto-run.
  run_demo "Bottling line" bottling bottling-line.json \
    line_bpm line_oee conv_hz fill_running energy_kw robot_picks robot_state
fi
if [[ $want == all || $want == generator ]]; then
  # Six generator nodes -> MQTT hub (embedded broker :1887): points good, topics published, data moving.
  MQTT_CHECK="1887 plant/gen/# plant/gen/tank_level plant/gen/pump_run plant/gen/pump_kw plant/gen/temperature plant/gen/humidity plant/gen/pressure plant/gen/flow plant/gen/load_kw plant/gen/energy_kwh plant/gen/wind plant/gen/battery_v plant/gen/heartbeat plant/gen/parts --moving plant/gen/tank_level plant/gen/pressure plant/gen/heartbeat plant/gen/temperature --timeout 40" \
  run_demo "Generator plant" none generator-plant.json \
    tank_level pump_run pump_kw temperature humidity pressure flow load_kw energy_kwh wind battery_v heartbeat parts
  MQTT_CHECK=""
fi
echo
if [[ ${#FAILED[@]} -eq 0 ]]; then echo "DEMO_SMOKE_OK"; else echo "FAILED: ${FAILED[*]}"; exit 1; fi

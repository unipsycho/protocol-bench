#!/usr/bin/env bash
# Run everything that can run without hardware: the workspace unit / integration tests, then the
# end-to-end smokes (each starts a real pb-service with the packs against fakes, in a private
# config dir so a running UI is never touched).
#
#   scripts/test-all.sh            # all of it
#   scripts/test-all.sh --quick    # unit tests only
#
# Needs: cargo, python3 (smokes). Linux / macOS.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
QUICK=0; [[ "${1:-}" == "--quick" ]] && QUICK=1
FAILED=()
step() { echo; echo "=== $* ==="; }
run() { local name="$1"; shift; step "$name"; if "$@"; then echo "--- $name: ok"; else echo "--- $name: FAILED"; FAILED+=("$name"); fi; }

run "cargo test --workspace" cargo test --workspace
if [[ $QUICK -eq 0 ]]; then
  run "modbus smoke (TCP client + server)" ./scripts/modbus-smoke.sh
  run "modbus RTU over virtual serial pair" ./scripts/modbus-rtu-smoke.sh
  run "file follow smoke" ./scripts/files-smoke.sh
  run "snmp + las smoke" ./scripts/snmp-las-smoke.sh
fi

echo
if [[ ${#FAILED[@]} -eq 0 ]]; then echo "ALL OK"; else echo "FAILED: ${FAILED[*]}"; exit 1; fi

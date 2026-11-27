#!/usr/bin/env bash
# Build the dashboard browser app (egui -> wasm) into pb-dashboard/dist, which the pack embeds.
# One-time setup:  rustup target add wasm32-unknown-unknown ; cargo install trunk
set -euo pipefail
cd "$(dirname "$0")/../pb-dash-web"
trunk build --release --dist ../pb-dashboard/dist
ls -l ../pb-dashboard/dist

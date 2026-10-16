#!/usr/bin/env bash
# Check cruzado do lado Windows (única validação Windows possível no Mac).
# O tauri-build exige um compilador de recursos (llvm-rc) mesmo no check; sem ele no PATH, usa o
# shim de scripts/shim (que só cria o .res vazio; check não linka). O CI em windows-latest usa o real.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
if ! command -v llvm-rc >/dev/null 2>&1; then
  export PATH="$RELAY_ROOT/scripts/shim:$PATH"
fi
cargo check --target x86_64-pc-windows-msvc \
  -p relay-core -p relay-platform-windows -p relay-windows-agent -p relay-windows-service

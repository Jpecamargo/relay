#!/usr/bin/env bash
# Gera os tipos TS do relay-core (ts-rs) em packages/protocol-types/src/generated. Detalhes no item F2.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
export TS_RS_EXPORT_DIR="$RELAY_ROOT/packages/protocol-types/src/generated"
cargo test -p relay-core --lib export_bindings

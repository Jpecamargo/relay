#!/usr/bin/env bash
# Gera os tipos TS do relay-core (ts-rs) em packages/protocol-types/src/generated e o index.ts
# que reexporta tudo. Os arquivos gerados são commitados; `scripts/check.sh` falha se divergirem.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
OUT="$RELAY_ROOT/packages/protocol-types/src/generated"
export TS_RS_EXPORT_DIR="$OUT"
rm -rf "$OUT"
mkdir -p "$OUT"
cargo test -p relay-core --lib export_bindings
{
  echo "// Gerado por scripts/gen-types.sh. Não edite."
  for f in $(cd "$OUT" && ls *.ts | grep -v '^index\.ts$' | sort); do
    echo "export * from \"./${f%.ts}\";"
  done
} > "$OUT/index.ts"

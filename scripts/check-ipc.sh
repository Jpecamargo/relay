#!/usr/bin/env bash
# Os dois backends Tauri precisam registrar exatamente os comandos de COMMANDS (bridge.ts).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Chaves de COMMANDS: linhas `  nome: "nome",` entre `export const COMMANDS = {` e `} as const`.
expected="$(awk '/^export const COMMANDS = \{/{f=1;next} /^\} as const/{f=0} f && /^  [a-z_]+:/{sub(/:.*/,"",$1);print $1}' packages/protocol-types/src/bridge.ts | sort -u)"
[ -n "$expected" ] || { echo "check-ipc: COMMANDS não encontrado em bridge.ts" >&2; exit 1; }

status=0
for lib in apps/mac/src-tauri/src/lib.rs apps/windows-agent/src-tauri/src/lib.rs; do
  # Último segmento de cada caminho dentro de generate_handler![ ... ].
  actual="$(awk '/generate_handler!\[/{f=1;next} f&&/\]/{f=0} f{gsub(/[ ,]/,"");n=split($0,p,"::");if(n)print p[n]}' "$lib" | sort -u)"
  missing="$(comm -23 <(echo "$expected") <(echo "$actual"))"
  extra="$(comm -13 <(echo "$expected") <(echo "$actual"))"
  [ -z "$missing" ] || { echo "check-ipc: $lib não registra: $(echo $missing)" >&2; status=1; }
  [ -z "$extra" ] || { echo "check-ipc: $lib registra fora de COMMANDS: $(echo $extra)" >&2; status=1; }
done
exit $status

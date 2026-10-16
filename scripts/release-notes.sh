#!/usr/bin/env bash
# Imprime as notas da versão do CHANGELOG.md (a seção "## X.Y.Z" da tag vX.Y.Z), sem o título.
# Vão para o corpo do GitHub Release e, daí, para o campo `notes` do latest.json que o updater lê
# (o modal "Nova versão do Relay" mostra isso em "O que mudou"). Uso: scripts/release-notes.sh v0.2.0
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
v="${1:-}"
v="${v#v}"
[[ -n "$v" ]] || { echo "uso: $0 vX.Y.Z" >&2; exit 1; }
awk -v v="$v" '
  /^## / { if (on) exit; on = ($2 == v) || ($2 == "[" v "]"); next }
  on { print }
' "$ROOT/CHANGELOG.md" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | sed '/./,$!d'

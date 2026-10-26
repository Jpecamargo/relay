#!/usr/bin/env bash
# Atualiza a versão do Relay em todos os lugares de uma vez:
#   [workspace.package] do Cargo.toml raiz (herdada pelos crates), tauri.conf.json do Mac e do agente
#   Windows, package.json dos apps e pacotes, e o Cargo.lock. Uso: scripts/bump-version.sh 0.2.0   (sem o "v")
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
v="${1:-}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "uso: $0 X.Y.Z (sem 'v')" >&2; exit 1; }
cd "$ROOT"

# Só a primeira linha `version = ` (a de [workspace.package]); as deps não são tocadas.
awk -v v="$v" '!d && /^version = "/ { print "version = \"" v "\""; d=1; next } { print }' Cargo.toml > Cargo.toml.new
mv Cargo.toml.new Cargo.toml

for f in apps/mac/src-tauri/tauri.conf.json apps/windows-agent/src-tauri/tauri.conf.json; do
  awk -v v="$v" '!d && /^  "version": "/ { print "  \"version\": \"" v "\","; d=1; next } { print }' "$f" > "$f.new"
  mv "$f.new" "$f"
done

for f in apps/mac/package.json apps/windows-agent/package.json packages/protocol-types/package.json packages/ui/package.json; do
  awk -v v="$v" '!d && /^  "version": "/ { print "  \"version\": \"" v "\","; d=1; next } { print }' "$f" > "$f.new"
  mv "$f.new" "$f"
done

# Regrava a versão dos crates do workspace no Cargo.lock sem buscar nada na rede.
cargo update --workspace --offline >/dev/null
echo "versão $v aplicada. Revise com git diff, commite e crie a tag v$v."

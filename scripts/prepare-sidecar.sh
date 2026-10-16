#!/usr/bin/env bash
# Prepara o sidecar do instalador Windows: compila relay-service e o copia para
# apps/windows-agent/src-tauri/binaries/relay-service-<target-triple>[.exe], nome exigido pelo
# `externalBin` do tauri.conf.json. Roda ANTES do `tauri build` (ver docs/distribuicao-windows.md).
#
#   prepare-sidecar.sh [triple]          build release real (windows-latest / Git Bash)
#   prepare-sidecar.sh --stub            só cria arquivos vazios, para check/clippy sem compilar
#                                        (o tauri-build exige o arquivo mesmo no `cargo check`)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/apps/windows-agent/src-tauri/binaries"
WIN_TRIPLE="x86_64-pc-windows-msvc"
mkdir -p "$DEST"

sidecar_name() { # $1 = triple
  case "$1" in *windows*) echo "relay-service-$1.exe" ;; *) echo "relay-service-$1" ;; esac
}

if [ "${1:-}" = "--stub" ]; then
  host="$(rustc -vV | sed -n 's/^host: //p')"
  for t in "$WIN_TRIPLE" "$host"; do
    f="$DEST/$(sidecar_name "$t")"
    [ -e "$f" ] || : > "$f"
  done
  exit 0
fi

triple="${1:-$WIN_TRIPLE}"
cd "$ROOT"
cargo build --release -p relay-windows-service --target "$triple"
out="${CARGO_TARGET_DIR:-$ROOT/target}/$triple/release"
case "$triple" in *windows*) bin="$out/relay-service.exe" ;; *) bin="$out/relay-service" ;; esac
cp "$bin" "$DEST/$(sidecar_name "$triple")"
echo "sidecar pronto: $DEST/$(sidecar_name "$triple")"

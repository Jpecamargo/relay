#!/usr/bin/env bash
# Põe o pacote oficial do VB-CABLE (VB-Audio, donationware) em apps/windows-agent/src-tauri/vbcable/,
# de onde o instalador NSIS o oferece (installer-hooks.nsh). Pacote sem modificação, conferido pelo
# SHA-256: a licença (vb-audio.com/Services/licensing.htm) permite embutir assim, com o usuário
# sabendo de onde vem. Chamado pelo prepare-sidecar.sh.
#
#   prepare-vbcable.sh          baixa e extrai (release)
#   prepare-vbcable.sh --stub   só cria a pasta (o tauri-build exige o recurso; o hook pula sem o setup)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/apps/windows-agent/src-tauri/vbcable"
URL="https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip"
SHA256="b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb"

mkdir -p "$DEST"
if [ "${1:-}" = "--stub" ]; then
  [ -e "$DEST/LEIA.txt" ] || echo "VB-CABLE não embutido (build local)." > "$DEST/LEIA.txt"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL -o "$tmp/pack.zip" "$URL"
if command -v sha256sum >/dev/null; then got=$(sha256sum "$tmp/pack.zip" | cut -d' ' -f1)
else got=$(shasum -a 256 "$tmp/pack.zip" | cut -d' ' -f1); fi
if [ "$got" != "$SHA256" ]; then
  echo "VB-CABLE: SHA-256 inesperado ($got). A VB-Audio publicou outro pacote: confira e atualize URL/SHA256." >&2
  exit 1
fi
rm -rf "$DEST" && mkdir -p "$DEST"
if command -v unzip >/dev/null; then unzip -q "$tmp/pack.zip" -d "$DEST"
else powershell -NoProfile -Command "Expand-Archive -LiteralPath '$(cygpath -w "$tmp/pack.zip")' -DestinationPath '$(cygpath -w "$DEST")'"; fi
echo "VB-CABLE pronto: $DEST"

#!/usr/bin/env bash
# Higiene do target/: roda sozinho (launchd, ver scripts/install-target-hygiene.sh) ou à mão.
#   sempre:          apaga */incremental e bundles antigos
#   > SOFT (15 GB):  apaga também release/ e o target cruzado do Windows
#   > HARD (25 GB):  cargo clean completo
# Não faz nada se houver cargo/rustc rodando (não quebra um build em andamento).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
SOFT_GB="${RELAY_TARGET_SOFT_GB:-15}"
HARD_GB="${RELAY_TARGET_HARD_GB:-25}"
export PATH="$HOME/.cargo/bin:$PATH"

log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*"; }
size_gb() { du -sk "$TARGET" 2>/dev/null | awk '{printf "%d", $1/1024/1024}'; }

[ -d "$TARGET" ] || { log "sem $TARGET"; exit 0; }
if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
  log "build em andamento; pulando"
  exit 0
fi

before=$(du -sh "$TARGET" | cut -f1)
rm -rf "$TARGET"/*/incremental "$TARGET"/*/*/incremental
find "$TARGET" -path '*/bundle/*' -maxdepth 4 -mtime +7 -exec rm -rf {} + 2>/dev/null || true

if [ "$(size_gb)" -ge "$HARD_GB" ]; then
  log "acima de ${HARD_GB} GB: cargo clean"
  (cd "$ROOT" && CARGO_TARGET_DIR="$TARGET" cargo clean)
elif [ "$(size_gb)" -ge "$SOFT_GB" ]; then
  log "acima de ${SOFT_GB} GB: removendo release/ e x86_64-pc-windows-msvc/"
  rm -rf "$TARGET/release" "$TARGET/x86_64-pc-windows-msvc"
fi

# Worktrees de agentes: nunca devem ter target/ próprio.
find "$ROOT/.team-work/worktrees" -mindepth 2 -maxdepth 2 -name target -type d -exec rm -rf {} + 2>/dev/null || true

log "target: $before -> $(du -sh "$TARGET" 2>/dev/null | cut -f1)"

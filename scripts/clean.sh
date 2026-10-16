#!/usr/bin/env bash
# Libera disco: cargo clean, incrementais residuais e target/node_modules de worktrees órfãos.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
cargo clean
rm -rf "$CARGO_TARGET_DIR"/*/incremental
WT="$RELAY_MAIN/.team-work/worktrees"
if [ -d "$WT" ]; then
  for d in "$WT"/*/; do
    [ -d "$d" ] || continue
    # Órfão = pasta que o git não lista mais como worktree.
    if ! git -C "$RELAY_MAIN" worktree list --porcelain | grep -qx "worktree ${d%/}"; then
      echo "órfão: $d"
    fi
    rm -rf "$d/target" "$d/node_modules"
  done
fi
echo "target/: $(du -sh "$CARGO_TARGET_DIR" 2>/dev/null | cut -f1)"

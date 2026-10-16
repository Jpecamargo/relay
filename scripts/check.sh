#!/usr/bin/env bash
# Validação completa do repositório. Termina com exit 0 só se tudo passar.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/check-windows.sh
pnpm install --frozen-lockfile
pnpm -r typecheck
pnpm -r build
pnpm -r test

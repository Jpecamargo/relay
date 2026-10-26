#!/usr/bin/env bash
# Validação completa do repositório. Termina com exit 0 só se tudo passar.
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$RELAY_ROOT"
cargo fmt --check
bash scripts/prepare-sidecar.sh --stub
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/check-windows.sh
pnpm install --frozen-lockfile
bash scripts/check-ipc.sh
# Tipos TS gerados precisam estar commitados e em dia com o Rust.
bash scripts/gen-types.sh >/dev/null
git diff --exit-code -- packages/protocol-types/src/generated
test -z "$(git ls-files --others --exclude-standard -- packages/protocol-types/src/generated)"
pnpm -r typecheck
pnpm -r build
pnpm -r test

# Sourced pelos scripts. Fixa o PATH do Node e o CARGO_TARGET_DIR único (target/ do repo principal,
# compartilhado entre worktrees para o disco não crescer). Funciona também fora de worktree.
export PATH="$HOME/.nvm/versions/node/v24.13.0/bin:$PATH"
RELAY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELAY_MAIN="$(dirname "$(git -C "$RELAY_ROOT" rev-parse --path-format=absolute --git-common-dir)")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$RELAY_MAIN/target}"

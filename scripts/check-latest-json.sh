#!/usr/bin/env bash
# Confere um latest.json do updater: tem a versão da tag e as três plataformas (com url e assinatura).
# Uso: scripts/check-latest-json.sh vX.Y.Z caminho/latest.json
# Chamado pelo job `verify-latest` do release.yml, antes do `publish`: Mac e PC precisam sair juntos,
# senão um lado se atualiza e o outro não acha a versão (e os dois ficam diferentes).
set -euo pipefail
tag="${1:?uso: check-latest-json.sh vX.Y.Z latest.json}"
file="${2:?uso: check-latest-json.sh vX.Y.Z latest.json}"
want="${tag#v}"

got=$(jq -r '.version // empty' "$file")
if [ "${got#v}" != "$want" ]; then
  echo "latest.json está na versão '${got:-?}', esperado $want" >&2
  exit 1
fi

missing=$(jq -r '. as $j
  | ["darwin-aarch64", "darwin-x86_64", "windows-x86_64"]
  | map(select(($j.platforms[.].url // "") == "" or ($j.platforms[.].signature // "") == ""))
  | join(" ")' "$file")
if [ -n "$missing" ]; then
  echo "latest.json sem plataforma(s) (ou sem url/assinatura): $missing" >&2
  exit 1
fi
echo "latest.json ok: $want com darwin-aarch64, darwin-x86_64 e windows-x86_64"

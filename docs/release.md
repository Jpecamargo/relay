# Release

Release por tag: `.github/workflows/release.yml` dispara em `v*` e publica **um único** GitHub Release
(`draft=false`, notas = seção da versão no `CHANGELOG.md`) com:

- Mac: `Relay_<v>_aarch64.dmg` e `Relay_<v>_x64.dmg` (+ `.app.tar.gz` e `.sig` do updater);
- Windows: instalador NSIS do agente (`Relay Agent_<v>_x64-setup.exe`, com o `relay-service` como sidecar) + `.sig`;
- `latest.json` com `darwin-aarch64`, `darwin-x86_64` e `windows-x86_64`. É o endpoint dos dois apps:
  `https://github.com/Jpecamargo/relay/releases/latest/download/latest.json`.

Jobs: `create-release` (cria o release) -> `mac` (matrix aarch64 e x86_64, em sequência) -> `windows`.
Em sequência de propósito: cada build regrava o `latest.json`, e em paralelo um apagaria a entrada do outro.

## Notas da versão

As notas vêm do `CHANGELOG.md` (seção `## X.Y.Z`, extraída por `scripts/release-notes.sh vX.Y.Z`), vão para o corpo do Release
e para `releaseBody` do tauri-action; assim o `latest.json` traz `notes`, que o modal "Nova versão do Relay" mostra em "O que mudou".
Sem a seção, o workflow cai nas notas geradas do GitHub.

## Cortar uma release

    # renomeie "Não publicado" no CHANGELOG.md para 0.2.0
    scripts/bump-version.sh 0.2.0      # sem o "v"
    git diff                           # Cargo.toml, Cargo.lock, os dois tauri.conf.json
    git commit -am "chore: v0.2.0"
    git tag v0.2.0
    git push origin main v0.2.0        # a tag dispara o workflow

`bump-version.sh` atualiza `[workspace.package].version` (herdada por todos os crates), a `version` dos
dois `tauri.conf.json` e o `Cargo.lock`. A tag deve bater com a versão. Se o workflow falhar, apague o
release e a tag (`gh release delete vX.Y.Z --cleanup-tag`), corrija e refaça.

A versão de protocolo (`PROTOCOL_VERSION` no core) é independente: versões diferentes do app continuam
se falando, e protocolo incompatível é avisado pelo core na conexão.

## Secrets (Settings > Secrets and variables > Actions)

Obrigatórios (já cadastrados):

| Secret | Uso |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | chave do updater (`~/.tauri/relay.key`, fora do repo) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | senha da chave (`~/.tauri/relay.key.password`) |

A pubkey correspondente está em `plugins.updater.pubkey` dos dois `tauri.conf.json`. Perder a chave privada
impede o updater de atualizar as instalações existentes (elas só aceitam atualização assinada por ela): faça
backup de `~/.tauri/relay.key` e da senha. Para trocar: `pnpm --filter @relay/mac exec tauri signer generate
-w ~/.tauri/relay.key`, atualize a pubkey nos dois confs e os dois secrets.

Opcionais. O workflow só os usa se existirem; sem eles o build sai **sem assinatura**:

| Secret | Uso |
|---|---|
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` | assina o app Mac (.p12 em base64) |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarização (senha específica de app); só com os três |
| `WINDOWS_CERTIFICATE` (+ `WINDOWS_CERTIFICATE_PASSWORD`) | .pfx em base64 e sua senha; assina o NSIS |

Detalhes de assinatura em `docs/distribuicao-mac.md` e `docs/distribuicao-windows.md`.

## Sem assinatura de código

Mac: o Gatekeeper bloqueia na primeira abertura (botão direito > Abrir, ou
`xattr -dr com.apple.quarantine /Applications/Relay.app`). Windows: o SmartScreen avisa; "Mais informações >
Executar assim mesmo".

## Build local

Com `createUpdaterArtifacts=true`, `tauri build` local sem a chave falha no fim (o bundle sai, mas termina com
"A public key has been found, but no private key"). Duas saídas:

    # assinar os artefatos do updater
    export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/relay.key)"
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.tauri/relay.key.password)"

    # ou desligar os artefatos do updater só nesse build
    pnpm --filter @relay/mac tauri build --debug --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'

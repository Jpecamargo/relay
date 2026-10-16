# Distribuição do Mac

Bundle `Relay.app` + `.dmg`, **sem App Sandbox** (o Relay usa event tap, `CGEventPost` e rede local),
com hardened runtime. `LSUIElement=true` (sem ícone no Dock). Mínimo: macOS 13.0.

Arquivos: `apps/mac/src-tauri/{tauri.conf.json, Info.plist, Entitlements.plist}`.

## Build local (sem assinatura)

    source scripts/env.sh
    pnpm --filter @relay/mac tauri build --debug

Saída em `$CARGO_TARGET_DIR/debug/bundle/{macos,dmg}`. `signingIdentity` é `null`: sem certificado, o
bundle sai sem assinatura. Release só no CI (tag `v*`, GitHub Actions, D32).

## Assinatura e notarização (variáveis de ambiente)

O Tauri lê estas variáveis; nada fica no repositório.

| Variável | Uso |
|---|---|
| `APPLE_SIGNING_IDENTITY` | ex.: `Developer ID Application: Nome (TEAMID)`; ativa a assinatura |
| `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` | `.p12` em base64 + senha (CI importa no keychain) |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | notarização por Apple ID e senha específica de app |
| `APPLE_API_ISSUER` / `APPLE_API_KEY` / `APPLE_API_KEY_PATH` | alternativa: chave de API do App Store Connect |

Com `APPLE_SIGNING_IDENTITY` e as credenciais de notarização definidas, `tauri build` assina com hardened
runtime e o `Entitlements.plist`, envia à notarização e grampeia o ticket. Conferir:

    codesign --verify --deep --strict --verbose=2 Relay.app
    spctl --assess --type execute --verbose Relay.app
    xcrun stapler validate Relay.dmg

## Updater

Endpoint (D32): `https://github.com/Jpecamargo/relay/releases/latest/download/latest.json`.
Hoje `plugins.updater.pubkey` é um **placeholder** e `bundle.createUpdaterArtifacts` é `false`: o app
compila, mas `check_updates` falha até a chave existir. Para ligar:

1. Gerar o par de chaves (guarde a privada fora do repo):

       pnpm --filter @relay/mac tauri signer generate -w ~/.tauri/relay.key

2. Colar o conteúdo de `relay.key.pub` em `plugins.updater.pubkey` no `tauri.conf.json`.
3. Trocar `bundle.createUpdaterArtifacts` para `true`.
4. No CI, definir os secrets `TAURI_SIGNING_PRIVATE_KEY` (e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).
   O build passa a gerar `Relay.app.tar.gz` + `.sig`; a action `tauri-apps/tauri-action` publica o
   `latest.json` no release.

Comandos Tauri: `check_updates` devolve a versão nova (ou `null`) e emite `relay:update-available`;
`install_update` baixa, instala e reinicia (exige `check_updates` antes).

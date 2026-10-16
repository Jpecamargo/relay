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
A `pubkey` real está em `plugins.updater.pubkey` e `bundle.createUpdaterArtifacts` é `true`. A chave
privada fica fora do repo em `~/.tauri/relay.key` (senha em `~/.tauri/relay.key.password`) e, no CI, nos
secrets `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. O build gera `Relay.app.tar.gz` +
`.sig` e o `tauri-apps/tauri-action` publica o `latest.json` (ver `docs/release.md`).

Build local exige a chave ou desligar os artefatos do updater (ver "Build local" em `docs/release.md`).

Fluxo (igual ao do yagni): o front usa `@tauri-apps/plugin-updater` e `plugin-process` (hook `useUpdater` em `@relay/ui`),
só no app instalado. O painel do menu bar é o dono do timer (15 s após abrir, depois a cada 6 h); ao achar versão nova avisa a janela
principal (`relay:update-available`), que repete a checagem, mostra "Atualização X disponível" na sidebar e o modal; o download só
começa em "Atualizar e reiniciar".

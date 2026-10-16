# Distribuição Windows

Instalador NSIS per-machine (`Relay Agent_<versão>_x64-setup.exe`) que instala o agente (bandeja),
o serviço `RelayService` (SYSTEM) e as regras de firewall. O build oficial roda no GitHub Actions
(`windows-latest`, D32); não há máquina Windows local.

## Build (Windows ou windows-latest, Git Bash)

A ordem importa: o `tauri build` embute o sidecar, então ele precisa existir antes.

```bash
# 1. relay-service release -> apps/windows-agent/src-tauri/binaries/relay-service-x86_64-pc-windows-msvc.exe
bash scripts/prepare-sidecar.sh
# 2. frontend + agente + instalador
pnpm install --frozen-lockfile
pnpm --filter @relay/windows-agent exec tauri build
```

O `externalBin` (`binaries/relay-service`) exige o sufixo do target triple no nome do arquivo;
o Tauri o instala como `relay-service.exe` ao lado do agente (`$INSTDIR`), que é o que o
`installer-hooks.nsh` chama. A pasta `binaries/` não é versionada.

Para `cargo check`/`clippy` sem compilar o serviço, `bash scripts/prepare-sidecar.sh --stub` cria
arquivos vazios (o `tauri-build` exige o arquivo); `check.sh` e `check-windows.sh` já fazem isso.

Saída: `target/release/bundle/nsis/`.

## O que o instalador faz (`installer-hooks.nsh`)

- Pós-instalação: `relay-service.exe install` (cria e inicia `RelayService`) e regras de entrada
  `Relay` no firewall, só no perfil **private**: TCP 47800 e 47804, UDP 47801-47803.
- Pré-desinstalação: `sc stop RelayService`, `relay-service.exe uninstall` e remoção das regras.
- WebView2: `embedBootstrapper` (baixa o bootstrapper embutido se faltar).

## VB-CABLE (áudio do Mac no PC)

Driver de terceiros, instalado à parte: baixar em vb-audio.com/Cable, extrair e rodar
`VBCABLE_Setup_x64.exe` como administrador, reiniciar. Depois o agente lista "CABLE Input" como
saída virtual. Sem ele o áudio do Mac não toca no PC (o resto funciona).

## Assinatura e SmartScreen

Hoje o instalador **não é assinado** (`certificateThumbprint: null`, D32): o SmartScreen mostra
"Windows protegeu seu computador"; clicar em "Mais informações" e "Executar assim mesmo".
Com certificado (OV/EV ou Azure Trusted Signing), configurar em `bundle.windows` do
`tauri.conf.json` ou por `signCommand`, por exemplo:

```json
"windows": { "signCommand": "signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 /f cert.pfx /p %CERT_PASSWORD% %1" }
```

Assine também o sidecar `relay-service.exe` antes do `tauri build` (o `signCommand` do Tauri cobre
o instalador e o executável do app). OV só limpa o SmartScreen após reputação acumulada; EV
limpa de imediato.

## Updater (D32)

`plugins.updater` aponta para
`https://github.com/Jpecamargo/relay/releases/latest/download/latest.json` (TLS nativo).
`pubkey` real e `createUpdaterArtifacts=true`; a chave privada está em `~/.tauri/relay.key` (fora do repo) e
nos secrets do CI (ver `docs/release.md`, inclusive como buildar local sem a chave). Fluxo: o front da janela do agente usa `@tauri-apps/plugin-updater` e `plugin-process` (hook `useUpdater` em `@relay/ui`),
com o modal "Nova versão do Relay" e o bloco Sobre nos Ajustes. O comando `agent_set_update_available` mostra/tira da bandeja o
item "Atualização X disponível", que abre a janela com o modal.

## Verificação pós-instalação

```powershell
Get-Service RelayService                      # Status: Running, StartType: Automatic
sc.exe qc RelayService                        # conta LocalSystem
Get-NetFirewallRule -DisplayName Relay | Get-NetFirewallPortFilter   # 47800/47804 TCP, 47801-47803 UDP
Get-NetFirewallRule -DisplayName Relay | Select Profile              # Private
Get-Content "$env:ProgramData\Relay\logs\service.log" -Tail 50
```

## Logs do serviço

`%ProgramData%\Relay\logs\service.log` (serviço) e `helper.log` (helper de sessão, gravado só se o
usuário da sessão tiver permissão de escrita na pasta). Rotação por tamanho: ao passar de 2 MB o
arquivo vira `.log.1` (um backup).

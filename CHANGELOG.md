# Changelog

Cada versão publicada ganha uma seção `## X.Y.Z`; o texto dela vira as notas do GitHub Release e o
"O que mudou" do modal de atualização (`scripts/release-notes.sh`). Ao cortar uma versão, renomeie
"Não publicado" para o número e abra uma nova seção "Não publicado" acima.

## Não publicado

## 0.1.1

- Nova interface no Mac: painel do menu bar mais enxuto e translúcido, ícone próprio na barra de menus e uma janela "Relay" com barra lateral (visão geral, telas, teclado, áudio, clipboard, conexão e geral).
- Editor de telas com ímã (encaixe nas bordas e alinhamentos; Option desliga) e orientação por monitor, com detecção automática de monitor girado no Windows.
- Movimento do mouse mais rápido: o canal rápido acorda por evento (ida e volta ~61 µs em loopback, antes ~231 µs) e o app parado não gasta CPU.
- Atualizações do app e do agente agora seguem o fluxo de confirmação: o Relay procura sozinho ao abrir e a cada 6 horas, mostra o que mudou e só baixa quando você clica em "Atualizar e reiniciar".
- Log em arquivo nos dois apps (`~/Library/Logs/Relay/relay.log` no Mac, `%LOCALAPPDATA%\Relay\logs\agent.log` no Windows).
- As métricas do PC aparecem na Visão geral da janela principal.

## 0.1.0

- Primeira versão: o teclado e o trackpad do Mac controlam o PC Windows na mesma rede, com pareamento por código e tráfego cifrado.
- Transição de foco entre as máquinas pela borda da tela, com layout de telas editável.
- Clipboard (texto, imagens e arquivos) e áudio entre o Mac e o PC.
- App na barra de menus do Mac e agente na bandeja do Windows.

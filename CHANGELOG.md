# Changelog

Cada versão publicada ganha uma seção `## X.Y.Z`; o texto dela vira as notas do GitHub Release e o
"O que mudou" do modal de atualização (`scripts/release-notes.sh`). Ao cortar uma versão, renomeie
"Não publicado" para o número e abra uma nova seção "Não publicado" acima.

## Não publicado

- O app do Mac passa a ter assinatura fixa: depois de conceder Acessibilidade e Monitoramento de Entrada uma última vez, as atualizações não pedem mais as permissões.

## 0.1.3

- O instalador do Windows oferece instalar o VB-CABLE (VB-Audio, donationware), necessário para o microfone do Mac no PC.
- Som do PC no Mac funciona sem configurar nada: o padrão passa a ser o sistema inteiro (antes só o Discord). Configs antigas migram sozinhas.
- Ao voltar para o Mac, o cursor não fica mais indo e voltando na borda: a borda de chegada só conta depois de o cursor se afastar dela.
- Pacotes de mouse e controle saem do Mac com prioridade de voz no Wi-Fi (menos atraso em rede sem fio).
- Mensagens de áudio mais claras: o VB-CABLE é necessário só para o microfone do Mac no PC.

## 0.1.2

- Menu da barra de menus mais útil: "Saída de som" para trocar o dispositivo de áudio do PC, "Ir para" uma tela específica, "Capturar tela do PC" e os atalhos reais mostrados ao lado de cada ação.
- O cursor não "agarra" mais na borda: a troca de tela acontece ao empurrar alguns pontos além dela, sem espera, e chega ao PC mais rápido (mediana de 3,4 ms para 0,7 ms). A intensidade fica em Ajustes › Transição › Resistência da borda.
- Mexer no volume não mostra mais o aviso "Comando executado".
- Ícone do Mac redesenhado na grade da Apple (cantos e tamanho iguais aos dos outros apps).

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

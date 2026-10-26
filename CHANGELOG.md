# Changelog

Cada versão publicada ganha uma seção `## X.Y.Z`; o texto dela vira as notas do GitHub Release e o
"O que mudou" do modal de atualização (`scripts/release-notes.sh`). Ao cortar uma versão, renomeie
"Não publicado" para o número e abra uma nova seção "Não publicado" acima.

## Não publicado

## 0.1.16

- Novo submenu "Layout" no menu da barra: troca o perfil de layout das telas em uso sem abrir o editor. Aparece quando há dois perfis ou mais.

## 0.1.15

- Volta a dar para usar o teclado do Mac com o mouse do PC (e o mouse do Mac com o teclado do PC). Na 0.1.13, qualquer movimento do mouse do PC tirava o controle do Mac e o teclado voltava a digitar no Mac. Agora a máquina só toma o controle quando o mouse dela atravessa a borda para a outra.

## 0.1.14

- Windows: o mouse e o teclado do Mac voltam a funcionar em janelas de administrador, como o Gerenciador de Tarefas, e na tela de confirmação do UAC. O agente nunca conseguia falar com o serviço do Relay (permissão incompleta no canal entre os dois) e injetava o input sem privilégio, que o Windows ignora nessas janelas.

## 0.1.13

- Mac e PC agora valem o mesmo: quem mexeu por último manda. Se o Mac está controlando o PC e você pega o mouse ou o teclado do PC, o PC assume na hora e o cursor do Mac volta para ele. O mesmo vale ao contrário: com o mouse do PC no Mac, mexer no trackpad ou no teclado do Mac devolve o controle ao Mac. Antes era preciso trazer o cursor de volta pela borda.
- Editor de telas: o ímã não encaixa mais uma tela por cima de outra (por exemplo, a base de um monitor em pé por cima do MacBook). Soltar uma tela em cima de outra a empurra para o lado mais próximo.

## 0.1.12

- Perfis de telas: o Relay não cria mais um perfil sozinho para cada combinação de monitores. Começa com um perfil, "Principal", e no editor você cria, renomeia, escolhe e exclui os seus. O perfil em uso se adapta quando um monitor é ligado ou desligado: telas novas entram ao lado das que já estavam.
- Os perfis "Padrão" criados automaticamente pelas versões anteriores viram um só, "Principal", mantendo a disposição mais completa.

## 0.1.11

- Som do PC no Mac: ao ligar, o PC fica mudo sozinho e o som sai só no Mac; ao desligar, o PC volta a tocar. Não é mais preciso trocar a saída de som no Windows. Se você já tinha deixado o PC mudo, ele continua mudo ao desligar.
- Som do PC no Mac: sem cortes quando o PC fica em silêncio entre um som e outro. Antes, cada pausa esvaziava o buffer no Mac e aumentava o atraso, e o som voltava picotado.
- Se o Relay fechar inesperadamente com o PC mudo, o som do PC volta na próxima vez que o agente abrir.

## 0.1.10

- Telas: com um segundo monitor no Mac, as telas não se sobrepõem mais no editor. As telas do Mac seguem a disposição do macOS, cada uma no tamanho real, e perfis já salvos com sobreposição são corrigidos ao abrir.
- Cursor: a checagem de que o cursor do Mac ficou parado com o foco no PC usa a posição real e roda no máximo a cada 100 ms (na 0.1.9 disparava a cada movimento).
- Som: registro no log quando o painel pede para ligar ou desligar o som, para diagnóstico.

## 0.1.9

- Com o foco no PC, se o macOS religar o trackpad ao cursor do Mac (os dois cursores andando juntos), o Relay percebe no próximo movimento, solta de novo e registra no log.
- O log do Mac registra cada pedido de "Som do PC no Mac" e avisa quando ele chega sem sessão, em vez de descartar em silêncio.
- Quando o PC recebe o mouse mas não consegue posicionar o cursor (o mouse "vai" mas não controla), o log do PC registra o motivo.

## 0.1.8

- Controle reverso (mouse do PC no Mac): ao voltar ao PC, o teclado volta junto; depois de usar o mouse do PC, o trackpad do Mac volta a atravessar para o PC.
- O PC registra no log por que uma borda não devolveu o mouse ao Mac, para diagnosticar travas.
- Mac e PC sempre na mesma versão: se um dos lados está mais atrás, ele se atualiza. Se o Mac está mais velho, o menu abre o aviso de atualização na hora, dizendo que é para ficar igual ao PC. Se o PC está mais velho, o Mac pede e o Relay Agent mostra o próprio "Atualizar e reiniciar" na tela do PC; o menu do Mac avisa "Atualizando o PC para a versão X…". Em ambos os casos a conexão volta sozinha depois que o app reinicia.
- A release só é publicada se o `latest.json` tiver Mac (Apple Silicon e Intel) e Windows na versão da tag.

## 0.1.7

- Volta do PC para o Mac corrigida: o cursor não volta mais sozinho para o PC logo depois de chegar ao Mac, e um movimento suave no trackpad já atravessa a borda (antes era preciso arrastar com força).

- O aviso de "PC na rede mas não responde" aponta a causa mais comum: o roteador isolando o Wi-Fi do cabo.

## 0.1.6

- Primeiro uso mais simples: sem PC pareado, o Mac abre direto numa tela "Conecte seu PC" com o código de 6 dígitos em caixas grandes (digite ou cole; envia sozinho ao completar). No Windows, o código aparece grande, com a contagem de expiração, no topo do agente.
- O serviço do Relay no Windows confere e recria as regras de firewall a cada início, então elas não somem mais.
- Quando o PC está na rede mas não responde, o menu do Mac explica e sugere atualizar o Relay Agent, em vez de ficar parado em "Conectando".
- Atualizador sem texto técnico: durante a publicação de uma versão, "Procurar atualizações" diz que não há versão nova em vez de mostrar erro, e a release só fica visível quando Mac e Windows estão prontos.

## 0.1.5

- O Relay funciona com a rede do Windows marcada como Pública: as regras de firewall do instalador passam a valer em qualquer tipo de rede, aceitando só aparelhos da mesma rede local. Antes, uma rede Pública deixava o Mac preso em "Conectando".

## 0.1.4

- Som do PC no Mac: se o instalador do VB-CABLE deixou o "CABLE Input" como saída padrão do Windows, o agente volta para a saída real (caixas/fone) enquanto o stream está ligado, em vez de capturar o próprio microfone.
- O volume no menu acompanha cada troca de saída de som do PC, não só a primeira.
- O ícone da barra de menus não pisca mais nas trocas de tela.
- Logs de diagnóstico do som do PC e das trocas de foco, nos dois apps.

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

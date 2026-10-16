# Bancada manual

Roteiro para validar à mão o que testes automáticos não cobrem. Duas máquinas: Mac (Relay.app) e PC
Windows pareados na mesma LAN. Marque cada item; anote versão, rede (cabo/Wi-Fi) e resultado.

## 0. Permissões (Mac, primeira execução)
- [ ] Sem permissões: o painel lista Acessibilidade e Monitoramento de Entrada como pendentes.
- [ ] "Abrir ajustes" leva ao painel certo; ao conceder, o painel atualiza sozinho (`relay:permissions`).
- [ ] Revogar com o app aberto: captura para e o painel avisa, sem travar o teclado do Mac.

## 1. Teclas presas na troca de foco
- [ ] Segurar Cmd (ou Shift) e cruzar a borda para o Windows e voltar: nenhuma tecla fica presa em nenhum lado.
- [ ] Segurar uma tecla de letra durante a troca: soltar depois não repete caractere.
- [ ] Soltar o modificador só depois de trocar: sem evento fantasma.

## 2. Secure Input
- [ ] Abrir um campo de senha no Mac (Secure Input ativo): o painel mostra o aviso (`relay:secure-input`).
- [ ] Fechar o campo: aviso some; captura de teclado volta ao normal.

## 3. Sleep e wake
- [ ] Mac dorme e acorda: sessão reconecta sozinha, sem reparear.
- [ ] PC dorme: Wake-on-LAN pelo painel (`wake_pc`) acorda; estado volta a "conectado".
- [ ] Durante o sleep o cursor nunca fica travado em um lado.

## 4. Troca de rede
- [ ] Trocar Wi-Fi para cabo (e vice-versa) no Mac: reconecta em segundos.
- [ ] Mudar de IP (DHCP): descoberta mDNS reencontra; endereço manual continua valendo.
- [ ] Rede cai por 30 s: estado "reconectando", sem perder o pareamento.

## 5. Monitor desconectado
- [ ] Desconectar um monitor no Mac com o editor de telas aberto: layout se reajusta, aviso de layout.
- [ ] Desconectar monitor do PC: o cursor não cai numa tela inexistente.
- [ ] Reconectar: layout salvo é restaurado.

## 6. Scroll do trackpad
- [ ] Rolagem vertical e horizontal no Windows com direção natural conforme a config.
- [ ] Inércia termina sem "rolagem fantasma"; pinça faz zoom nos apps compatíveis.

## 7. ABNT2
- [ ] Digitar no PC com teclado ABNT2: `ç`, `~`, `^`, `´`, `/`, `?`, `\`, `|` e a tecla extra saem certos.
- [ ] Acentos mortos (`~` + `a`) compõem corretamente.
- [ ] Atalhos remapeados (Cmd para Ctrl: C, V, X, Z, Tab, Q) funcionam.

## 8. Latência (relay-bench)
Detalhes em `docs/latencia.md`. Em cada rede (cabo e Wi-Fi), no PC:

    relay-bench echo --code 123456

No Mac:

    relay-bench send --peer <host-do-pc> --code 123456 --count 5000 --rate 500 --json bancada.json

- [ ] Registrar p50/p95/p99/max (µs) do canal rápido e do confiável.
- [ ] Comparar cabo e Wi-Fi; registrar, sem meta fixa.

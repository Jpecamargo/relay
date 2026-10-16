# Latência (relay-bench)

`relay-bench` mede o RTT (ida e volta) sobre a sessão real do `relay-core`: Noise no TCP, AEAD no
UDP, chaves efêmeras (nada é gravado em disco nem no Keychain). Cada canal é medido sozinho:

- **rápido** (UDP, o que leva MouseMove/Scroll): `MouseMove` com o id do pacote em `dx`/`dy`; o
  lado `echo` devolve o mesmo evento pelo próprio canal rápido.
- **confiável** (TCP): `Message::Input` com o id em `seq`; o `echo` devolve a mensagem.

O RTT inclui cifra/decifra nos dois sentidos e as threads do canal rápido. Relógio único
(`net::now_us`, o do processo que mede), então não depende de sincronizar as máquinas.

## Modos

    relay-bench echo --code 123456 [--port-control 47800] [--port-input 47801]
    relay-bench send --peer <host[:porta]> --code 123456 [--count 5000] [--rate 500] [--json arq.json]
    relay-bench loopback [--count 10000] [--rate 2000] [--json arq.json]

- `echo` escuta (em 0.0.0.0), aceita o pareamento com o código e reflete os pacotes. Ctrl+C encerra.
- `send` pareia com o código, abre a sessão e mede primeiro o canal rápido, depois o confiável.
  Antes de medir, aquece até o primeiro eco voltar (o agente aprende o endereço UDP do Mac pelo
  probe do heartbeat); se o UDP estiver bloqueado, falha em 15 s.
- `loopback` roda os dois papéis neste processo, em 127.0.0.1, portas efêmeras.
- Saída: p50, p95, p99 e max em µs por canal, mais enviados/recebidos/perdidos. `--json` grava o
  mesmo em arquivo (`mode`, `rate_hz`, `fast{...}`, `reliable{...}`, valores `*_us`).
- Limite do ritmo no canal confiável: o timer do tokio tem granularidade de 1 ms; acima de
  1000 Hz ele envia em rajadas. O canal rápido segura o ritmo com espera ativa no final.

## Medir no cabo e no Wi-Fi

1. Compile em release nas duas máquinas: `cargo build --release -p relay-latency-bench`
   (binário `relay-bench`; no Windows, `relay-bench.exe`). Libere as portas 47800/TCP e 47801/UDP
   no firewall do PC (ou use `--port-control`/`--port-input` nos dois lados).
2. No PC: `relay-bench echo --code 123456`.
3. No Mac: `relay-bench send --peer <ip-ou-nome-do-pc> --code 123456 --count 5000 --rate 500 --json cabo.json`.
4. Repita em cada rede (Mac e PC no mesmo switch/cabo, depois os dois no Wi-Fi), com a mesma
   contagem e ritmo. Rode uma segunda vez: a primeira costuma pegar caches frios.
5. Para estressar o Wi-Fi, repita com `--rate 125` e `--rate 1000`; compare p99 e perdidos.
6. Anote rede, banda (2,4/5 GHz), distância e carga na rede; registre na tabela abaixo.

No Wi-Fi o p99 e o max importam mais que o p50 (economia de energia do rádio e retransmissões
aparecem na cauda). Sem meta fixa para cabo/Wi-Fi; a única meta é o loopback abaixo.

## Resultados

Mac (Apple Silicon), release, 127.0.0.1 (`loopback`), sessão cifrada. RTT em µs.

| Execução | Canal | p50 | p95 | p99 | max | Perdidos |
|---|---|---|---|---|---|---|
| loopback, 10000 @ 2000 Hz | rápido | 221 | 316 | 340 | 364 | 0 |
| | confiável | 89 | 106 | 121 | 241 | 0 |
| loopback, 10000 @ 2000 Hz (2ª) | rápido | 232 | 322 | 338 | 358 | 0 |
| | confiável | 90 | 108 | 125 | 424 | 0 |
| loopback, 5000 @ 500 Hz | rápido | 254 | 333 | 340 | 794 | 0 |
| | confiável | 92 | 108 | 118 | 216 | 0 |
| echo + send (2 processos), 1000 @ 500 Hz | rápido | 257 | 335 | 343 | 408 | 0 |
| | confiável | 88 | 105 | 117 | 186 | 0 |

Meta do loopback (p50 do canal rápido < 500 µs): atendida (~220-260 µs).

Depois da otimização do canal rápido (thread de envio estaciona em `park` e o `push` a acorda, em
vez de dormir 100 µs; a recepção já bloqueava no socket com timeout), mesma máquina, release,
127.0.0.1, duas execuções cada (RTT em µs, canal rápido):

| Execução | p50 antes | p50 depois | p99 antes | p99 depois |
|---|---|---|---|---|
| loopback, 10000 @ 2000 Hz | 231-236 | 61-62 | 336-337 | 72-74 |
| loopback, 3000 @ 125 Hz | 263-267 | 134-135 | 343-345 | 239-241 |
| loopback, 1000 @ 30 Hz | 282-310 | 199-205 | 346-352 | 292-299 |

CPU do `relay-bench echo` parado (média de 10 s, `ps -o %cpu`): 2,0% antes, 0,0% depois.
Medições feitas com a máquina quieta; com outros builds rodando o p95/p99 sobe para milissegundos
em qualquer versão.

Em ritmo baixo o RTT ainda soma duas acordadas de thread a frio (envio e recepção, nos dois
sentidos), por isso 125 Hz e 30 Hz ficam acima do caso de 2000 Hz, onde a thread ainda está girando.
Antes da otimização o sono de 100 µs da thread de envio (o `thread::sleep` do macOS sai ~1,5x o
pedido) pesava em cada um dos dois sentidos. Em rede real esse custo é pequeno diante do RTT da
rede. Cabo e Wi-Fi: a medir na bancada (`docs/bancada.md`, seção 8).

## Travessia da borda (sensação de "agarrada")

Duas causas, ambas corrigidas:

1. **Tempo de espera na borda.** O cursor ficava preso até `dwell_ms` = 100 ms completar. Agora a
   troca de máquina é por pressão: soma dos deltas para fora da borda >= `edge_resistance_pt`
   (padrão 8 pt; Ajustes > Transição > "Resistência da borda": Nenhuma 0, Leve 8, Média 20).
   `dwell_ms` virou só um teto opcional (padrão 0 = sem teto). Vale também para o retorno no
   receptor. Zona morta de canto e modificador não mudaram.
2. **Laço do core dormindo.** Em foco local o laço do Mac dormia ~4 ms pedidos (~6 ms reais) e o
   evento da captura esperava o fim do sono. Agora o callback do tap (Mac) e o hook (Windows, sentido
   inverso) chamam `LoopWaker::wake` (`Thread::unpark`, sem alocação nem lock) e o laço dorme em
   `park_timeout`.

Medições (Mac, loopback, release do core, laço ocioso antes de cada cruzamento):

| O que | Antes | Depois |
|---|---|---|
| Evento que cruza a borda -> `FocusEnter` no agente (200 cruzamentos, `latencia_da_borda`) | p50 3444 us, p95 5509 us | p50 705 us, p95 803 us |
| Cursor empurrando 3 pt/evento a 125 Hz até sair (Mac ou retorno no PC) | 100 ms (dwell) | 3 eventos, 16 ms |
| Flick de 8 pt ou mais num evento | 100 ms | no primeiro evento |

`latencia_da_borda` é um teste `#[ignore]` em `apps/mac/src-tauri/src/core/tests.rs`:
`cargo test -p relay-mac latencia_da_borda -- --ignored --nocapture`; com `RELAY_NO_WAKE=1` a
captura simulada não acorda o laço (comportamento antigo). O caso de 3 pt/evento é o teste
`retorno_por_pressao_sai_em_16_ms_e_nao_em_100` (relógio sintético).

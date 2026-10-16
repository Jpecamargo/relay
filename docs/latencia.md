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

O canal rápido é mais lento que o TCP em loopback porque as threads do canal (envio e recepção)
fazem espera adaptativa: giram, cedem a CPU e dormem 100 µs quando ociosas; em ritmo baixo o
pacote espera o fim desse sono em cada um dos dois sentidos. Em rede real esse custo é pequeno
diante do RTT da rede. Cabo e Wi-Fi: a medir na bancada (`docs/bancada.md`, seção 8).

# Mapa de donos (fixado pelo F2)

Cada arquivo tem um item dono, que é o único a editá-lo. A primeira linha de cada módulo é
`//! dono: <item>`. O F2 deixou todos os arquivos declarados e compilando com stubs (assinaturas
públicas + corpo neutro, nunca `todo!()`): o item dono só preenche o corpo, ajusta as assinaturas
do **seu** arquivo se precisar e acrescenta testes. Arquivos novos dentro do diretório de um dono
único também são dele.

## Fechado pelo F2 (ninguém mais edita)
Mudar um destes pede ao PO/orquestrador. Se um tipo faltar, reporte em vez de editar.

| Arquivo | Conteúdo |
|---|---|
| `crates/core/src/lib.rs`, `crates/core/src/keymap/mod.rs` | declaração dos módulos |
| `crates/core/src/types/*` | tipos de domínio e protocolo (serde + ts-rs), `consts`, `Config` com padrões |
| `crates/core/src/platform.rs` | traits de plataforma e seus erros |
| `crates/platform-macos/src/lib.rs`, `crates/platform-windows/src/lib.rs` | declaração dos módulos |
| `packages/protocol-types/src/{bridge,index}.ts`, `src/generated/*` | cliente tipado e tipos gerados (`pnpm gen:types`) |

## relay-core
| Arquivo | Dono | Notas |
|---|---|---|
| `crates/core/src/protocol/` | C1 | codec rápido, frames, `SeqFilter` |
| `crates/core/src/crypto/` | C2 | SPAKE2, Noise KK, `UdpAead`, `MemoryKeyStore`, `SystemKeyStore` |
| `crates/core/src/net/` | C3 | sessão; `net/fast` sem tokio |
| `crates/core/src/discovery.rs` | C4 | mDNS e endereço manual |
| `crates/core/src/wol.rs` | C4 | Wake-on-LAN |
| `crates/core/src/keymap/tables.rs` | C5a | tabelas mac↔scan, mídia |
| `crates/core/src/keymap/translator.rs` | C5b | tradutor de atalhos |
| `crates/core/src/keymap/pressed.rs` | C5b | `PressedTracker` |
| `crates/core/src/keymap/hotkeys.rs` | C5b | `HotkeyMatcher` |
| `crates/core/src/layout/` | C6 | modelo em mm, perfis, avisos |
| `crates/core/src/transition.rs` | C7 | motor de transição |
| `crates/core/src/input_shaping/` | C8 | scroll, pinça, sensibilidade |
| `crates/core/src/config.rs` | C9 | persistência TOML (a struct está em `types/config.rs`) |
| `crates/core/src/engine/` | C10 | Controller, Receiver, CommandDispatcher |
| `crates/core/src/clipboard/` | C11a | sincronização de clipboard |
| `crates/core/src/transfer/` | C11b | transferência de arquivos |
| `crates/core/src/audio/` | C12 | pacote, jitter, deriva, conversões |
| `crates/latency-bench/` | B1 | bancada relay-bench |

## relay-platform-macos (tudo sob `cfg(target_os = "macos")`)
| Arquivo | Dono |
|---|---|
| `capture.rs`, `examples/tap_probe.rs` | M1 |
| `inject.rs`, `cursor.rs`, `permissions.rs`, `secure_input.rs`, `examples/cursor_probe.rs` | M2 |
| `displays.rs`, `system_events.rs`, `login_item.rs`, `dock.rs` | M3 |
| `audio.rs`, `examples/audio_loop.rs` | M4 |
| `clipboard.rs`, `files.rs` | M5 |

`DockInfo` e `HotCorners` vivem em `relay_core::types` (o core não depende do backend); `dock.rs`
só os lê do sistema.

## relay-platform-windows
Todos os módulos compilam em qualquer target. O que chama a API do Windows fica sob
`#[cfg(windows)]` dentro do arquivo; lógica pura (mapeamentos, parsers, conversões) fica sem cfg
e é testada no Mac com `cargo test -p relay-platform-windows`.

| Arquivo | Dono |
|---|---|
| `inject.rs`, `inject_map.rs`, `dpi.rs`, `mouse_accel.rs` | W1 |
| `hooks.rs`, `hooks_map.rs`, `cursor.rs` | W2 |
| `displays.rs`, `edid.rs`, `power.rs`, `shell.rs`, `foreground.rs`, `net.rs`, `keyboard.rs` | W3 |
| `audio_endpoints.rs` | W4a |
| `audio_stream.rs`, `audio_convert.rs` | W4b |
| `clipboard.rs`, `clipboard_formats.rs`, `explorer.rs` | W5 |
| `metrics.rs`, `screen_capture.rs` | W6 |
| `service.rs`, `pipe.rs`, `apps/windows-service/src/*` | W7 |

## Apps e pacotes TS
| Caminho | Dono |
|---|---|
| `apps/mac/src-tauri/src/*` (exceto abaixo), `apps/mac/src/main.tsx` | MAC1 |
| `apps/mac/src-tauri/src/core/*` (exceto `core/media/*`), `commands/{status,pairing,layout,config}.rs` | MI1 |
| `apps/mac/src-tauri/src/commands/controls.rs`, `commands/system.rs`, `tray.rs` | MI2 |
| `apps/mac/src-tauri/src/commands/audio.rs`, `commands/clipboard.rs`, `core/media/*` | MI3 |
| `apps/mac/src/panel/` | U2 |
| `apps/mac/src/editor/` | U1 |
| `apps/mac/src/settings/` | U3 |
| `apps/windows-agent/src-tauri/src/*` (exceto abaixo), `apps/windows-agent/src/main.tsx` | WA1 |
| `apps/windows-agent/src-tauri/src/core/*` (exceto `executor.rs` e `core/media/*`) | WI1 |
| `apps/windows-agent/src-tauri/src/core/executor.rs` | WI2 |
| `apps/windows-agent/src-tauri/src/core/media/*` | WI3 |
| `apps/windows-agent/src/` (exceto `main.tsx`) | WAU |
| `packages/ui/` | U0 |

Conflitos entre MAC1 e MI1/MI2/MI3 (e WA1 e WI1/WI2/WI3) em `apps/*/src-tauri/src` seguem a
tabela acima: MAC1/WA1 criam o casco; os de integração só preenchem os arquivos listados.

## Convenções do contrato
- Ids de correlação (`Command.id`, `offer_id`, `transfer_id`) são `u32`; `seq`, `ts_us` e tamanhos
  em bytes são `u64` (no TS viram `number`).
- Posições: `DisplayInfo.origin_px/size_px` e `FocusEnter.entry_px` usam o espaço de coordenadas
  da máquina dona da tela (Mac: pontos; Windows: pixels físicos). `scale` serve ao fallback de mm.
- `HotkeyAction::JumpTo(n)` é 1-based (1 = MacBook, D19). `KeyCombo.key` segue `KeyboardEvent.code`.
- `FileEntry.id` é o índice na lista de caminhos locais do emissor (`ClipboardBackend::local_paths`).
- `CommandOutput::Png` nunca vai no fio: o agente o converte em `ClipboardOffer`.
- Comandos Tauri: handlers com `#[tauri::command(rename_all = "snake_case")]` (nomes do glossário).
- `expires_at` do pareamento: epoch em milissegundos.

# Relay

KVM por software: o mesmo teclado e trackpad do Mac controlam um PC Windows na mesma LAN.
Tauri v2 nas duas pontas, Rust + React/TS (Vite). Spec em `docs/spec.pdf`; ambiente em `docs/dev.md`.

## Estrutura
| Caminho | Pacote | O que é |
|---|---|---|
| `crates/core` | `relay-core` | Protocolo, cripto, sessão, layout, keymap (Rust puro) |
| `crates/platform-macos` | `relay-platform-macos` | Backends macOS (`cfg(target_os = "macos")`) |
| `crates/platform-windows` | `relay-platform-windows` | Backends Windows (`cfg(windows)`) |
| `crates/latency-bench` | `relay-latency-bench` | Binário `relay-bench` (p50/p95/p99) |
| `apps/mac` | `@relay/mac` / `relay-mac` | App Tauri do Mac (`src-tauri` + React) |
| `apps/windows-agent` | `@relay/windows-agent` / `relay-windows-agent` | Agente Tauri do Windows |
| `apps/windows-service` | `relay-windows-service` | Serviço SYSTEM (`relay-service.exe`), sem UI |
| `packages/ui` | `@relay/ui` | Componentes React compartilhados |
| `packages/protocol-types` | `@relay/protocol-types` | Tipos TS gerados do core (`pnpm gen:types`) |

## Comandos
- `pnpm i && pnpm dev`: roda o app do Mac em modo dev (compila os pacotes compartilhados e abre o `tauri dev`).
- `pnpm dev:mock`: o mesmo, com backend simulado (3 telas, conectado), sem pedir permissões.
- `scripts/check.sh`: validação completa (fmt, clippy `-D warnings`, testes, check cruzado Windows, typecheck/build/test do pnpm). Deve terminar com exit 0.
- `scripts/check-windows.sh`: só o check cruzado do Windows (ver `docs/dev.md`).
- `scripts/target-hygiene.sh`: higiene do `target/` (incrementais sempre; >15 GB apaga release/ e o target do Windows; >25 GB `cargo clean`; pula se houver build rodando). `scripts/install-target-hygiene.sh` agenda no launchd (diário 12h + login; log em `~/Library/Logs/Relay/target-hygiene.log`; `--uninstall` remove).
- `scripts/clean.sh`: `cargo clean`, remove `incremental` e `target`/`node_modules` de worktrees órfãos em `.team-work/worktrees`.
- `pnpm install && pnpm -r typecheck && pnpm -r build && pnpm -r test`: lado TypeScript.
- `pnpm --filter @relay/mac tauri build --debug --bundles app`: gera `Relay.app`.

Todos os comandos cargo usam o `target/` único do repo principal; veja `docs/dev.md`.

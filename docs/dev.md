# Desenvolvimento

## Toolchain
- Rust 1.98 com targets `aarch64-apple-darwin` e `x86_64-pc-windows-msvc`, mais componentes `rustfmt` e `clippy`.
- Node 24 + pnpm: `export PATH="$HOME/.nvm/versions/node/v24.13.0/bin:$PATH"`.

## CARGO_TARGET_DIR único
Todos os builds usam `CARGO_TARGET_DIR=/Users/joaocamargo/Code/relay/target`, o `target/` do repo
principal, compartilhado entre worktrees para o disco não crescer. Os scripts em `scripts/` já
exportam isso (via `scripts/env.sh`, que resolve o repo principal a partir do worktree). Em comandos
manuais:

    export CARGO_TARGET_DIR=/Users/joaocamargo/Code/relay/target

Nunca crie `target/` dentro de um worktree. Evite `cargo build --release` local; prefira `cargo check` e `cargo test`.

Contenção de tamanho, no `Cargo.toml` raiz: `[profile.dev]` com `debug = "line-tables-only"` e
`incremental = false`; `[profile.dev.package."*"]` com `debug = false`; `[profile.release]` com
`strip`, `lto = "thin"` e `codegen-units = 1`. `scripts/clean.sh` libera o espaço.

## Dependências
Versões fixadas em `[workspace.dependencies]` (Cargo.toml raiz). Cada crate herda com
`dep.workspace = true`; dependência nova de um crate edita só o `Cargo.toml` dele.
`snow` roda sem `ring`/`aws-lc` (features `use-*` explícitas): C e asm quebram o check cruzado para Windows.

## Check cruzado do Windows
`scripts/check-windows.sh` roda `cargo check --target x86_64-pc-windows-msvc` para core, platform-windows,
agente e serviço. O `tauri-build` exige um `llvm-rc` mesmo no check; sem ele no PATH, o script usa
`scripts/shim/llvm-rc`, que só cria o `.res` vazio (check não linka). Não serve para build real. O CI em
`windows-latest` usa o compilador de recursos verdadeiro.

## Tauri
`@tauri-apps/cli` vem via pnpm (sem binário global): `pnpm --filter @relay/mac tauri build --debug --bundles app`.
Os pacotes npm `@tauri-apps/api` e `@tauri-apps/cli` ficam em `~2.11` para casar com a crate `tauri` 2.11.
O `build.rs` de cada app cria `dist/` vazio se faltar, porque `generate_context!` exige `frontendDist`.

import type { CommandResult } from "@relay/protocol-types";

// Códigos fixos de CommandResult.error (crates/core/src/types/command.rs, mod error_code).
const MENSAGENS: Record<string, string> = {
  timeout: "O PC não respondeu a tempo.",
  confirmation_required: "Ação precisa de confirmação.",
  vbcable_missing: "Áudio do Mac no PC indisponível: instale o VB-Cable no Windows.",
  virtual_output_missing: "Áudio do PC no Mac indisponível: falta a saída de áudio virtual no Windows.",
  unsupported: "Não suportado neste sistema.",
};

/** Erro de qualquer origem (string do Tauri, Error, outro) → texto. */
export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export function errorText(e: unknown): string {
  const raw = errorMessage(e);
  return MENSAGENS[raw] ?? raw;
}

export function isCommandResult(v: unknown): v is CommandResult {
  return typeof v === "object" && v !== null && "ok" in v && "id" in v;
}

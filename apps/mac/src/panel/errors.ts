import type { CommandResult } from "@relay/protocol-types";

// Códigos fixos de CommandResult.error (crates/core/src/types/command.rs, mod error_code).
const MENSAGENS: Record<string, string> = {
  timeout: "O PC não respondeu a tempo.",
  confirmation_required: "Ação precisa de confirmação.",
  vbcable_missing: "VB-Cable não está instalado no PC.",
  virtual_output_missing: "Saída de áudio virtual ausente no PC.",
  unsupported: "Não suportado neste sistema.",
};

export function errorText(e: unknown): string {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return MENSAGENS[raw] ?? raw;
}

export function isCommandResult(v: unknown): v is CommandResult {
  return typeof v === "object" && v !== null && "ok" in v && "id" in v;
}

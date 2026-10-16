import { describe, expect, test } from "vitest";
import { COMMANDS, EVENTS, invokeCommand, listenEvent } from "./index";

describe("bridge", () => {
  test("COMMANDS cobre os 29 comandos do Mac e os 7 do agente, cada nome igual à chave", () => {
    const entries = Object.entries(COMMANDS);
    expect(entries).toHaveLength(36);
    for (const [key, value] of entries) expect(value).toBe(key);
    expect(entries.filter(([k]) => k.startsWith("agent_"))).toHaveLength(7);
  });

  test("EVENTS tem os 11 eventos do glossário, todos com prefixo relay:", () => {
    const values = Object.values(EVENTS);
    expect(values).toHaveLength(11);
    for (const v of values) expect(v).toMatch(/^relay:/);
  });
});

// Só compila (não roda): confere a tipagem de invokeCommand e listenEvent.
export function typeChecks() {
  void invokeCommand("get_status");
  void invokeCommand("pair_with_code", { code: "123456" });
  void invokeCommand("send_command", { command: { Power: { action: "Lock", confirm: false } } });
  // @ts-expect-error argumento obrigatório ausente
  void invokeCommand("set_cursor_lock");
  // @ts-expect-error nome de comando inexistente
  void invokeCommand("nao_existe");
  void listenEvent("relay:secure-input", (p) => p.active);
}

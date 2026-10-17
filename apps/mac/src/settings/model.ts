// dono: U3. Regras embutidas, ações de atalho e validação do Config editado na janela.
import type { Config, HotkeyAction, HotkeyBinding, KeyCombo, KeymapRule } from "@relay/protocol-types";

/** Regras embutidas do tradutor (ids de crates/core/src/keymap/translator.rs, C5b). */
export interface BuiltinRule {
  id: string;
  from: string;
  to: string;
  /** Só vale em terminais do Windows. */
  terminal?: boolean;
}

const cmdCtrl = (key: string): BuiltinRule => ({ id: `cmd_${key.toLowerCase()}`, from: `⌘${key}`, to: `Ctrl+${key}` });

export const BUILTIN_RULES: ReadonlyArray<BuiltinRule> = [
  ...["C", "V", "X", "Z", "A", "S", "F", "T", "W", "N", "P", "R"].map(cmdCtrl),
  { id: "cmd_shift_z", from: "⌘⇧Z", to: "Ctrl+Y" },
  { id: "cmd_tab", from: "⌘Tab", to: "Alt+Tab" },
  { id: "cmd_q", from: "⌘Q", to: "Alt+F4" },
  { id: "cmd_left", from: "⌘←", to: "Home" },
  { id: "cmd_right", from: "⌘→", to: "End" },
  { id: "cmd_up", from: "⌘↑", to: "Ctrl+Home" },
  { id: "cmd_down", from: "⌘↓", to: "Ctrl+End" },
  { id: "opt_left", from: "⌥←", to: "Ctrl+←" },
  { id: "opt_right", from: "⌥→", to: "Ctrl+→" },
  { id: "cmd_backspace", from: "⌘⌫", to: "Apagar até o início da linha" },
  { id: "opt_backspace", from: "⌥⌫", to: "Ctrl+Backspace" },
  { id: "cmd_shift_4", from: "⌘⇧4", to: "Win+Shift+S" },
  { id: "cmd_shift_5", from: "⌘⇧5", to: "Win+Shift+S" },
  { id: "ctrl_cmd_q", from: "⌃⌘Q", to: "Win+L (bloquear)" },
  { id: "cmd_opt_esc", from: "⌘⌥Esc", to: "Ctrl+Shift+Esc" },
  { id: "cmd_space", from: "⌘Space", to: "Tecla Windows" },
  { id: "terminal_copy", from: "⌘C", to: "Ctrl+Shift+C", terminal: true },
  { id: "terminal_paste", from: "⌘V", to: "Ctrl+Shift+V", terminal: true },
  { id: "terminal_interrupt", from: "⌃C", to: "Ctrl+C", terminal: true },
];

/** Ações de atalho global editáveis (as `Command` ficam fora: não têm gravador por ação). */
export const HOTKEY_ACTIONS: ReadonlyArray<{ action: HotkeyAction; label: string }> = [
  { action: { JumpTo: 1 }, label: "Ir para a tela 1" },
  { action: { JumpTo: 2 }, label: "Ir para a tela 2" },
  { action: { JumpTo: 3 }, label: "Ir para a tela 3" },
  { action: "ToggleLock", label: "Travar cursor" },
  { action: "TogglePause", label: "Pausar compartilhamento" },
  { action: "ClipboardSendNow", label: "Enviar clipboard agora" },
  { action: "WakePc", label: "Acordar o PC" },
];

export const actionKey = (a: HotkeyAction): string => (typeof a === "string" ? a : JSON.stringify(a));

export const comboKey = (c: KeyCombo): string => `${[...c.modifiers].sort().join("+")}+${c.key}`;

export function isValidPairCode(code: string): boolean {
  return /^\d{6}$/.test(code);
}

/** Troca (ou remove, com `null`) o atalho de uma ação. */
export function setHotkey(list: HotkeyBinding[], action: HotkeyAction, combo: KeyCombo | null): HotkeyBinding[] {
  const k = actionKey(action);
  const rest = list.filter((b) => actionKey(b.action) !== k);
  return combo ? [...rest, { action, combo }] : rest;
}

/** Ações que usam a mesma combinação que outra, por chave de ação. */
export function hotkeyConflicts(list: HotkeyBinding[]): Map<string, string[]> {
  const byCombo = new Map<string, HotkeyBinding[]>();
  for (const b of list) byCombo.set(comboKey(b.combo), [...(byCombo.get(comboKey(b.combo)) ?? []), b]);
  const out = new Map<string, string[]>();
  for (const group of byCombo.values()) {
    if (group.length < 2) continue;
    for (const b of group) {
      const others = group.filter((o) => o !== b).map((o) => HOTKEY_ACTIONS.find((h) => actionKey(h.action) === actionKey(o.action))?.label ?? actionKey(o.action));
      out.set(actionKey(b.action), others);
    }
  }
  return out;
}

/** Atualiza uma regra por id; regra que volta ao padrão (ativa, sem from/to) some da lista. */
export function patchRule(rules: KeymapRule[], id: string, patch: Partial<KeymapRule>): KeymapRule[] {
  const cur = rules.find((r) => r.id === id) ?? { id, enabled: true, from: null, to: null };
  const next = { ...cur, ...patch };
  const rest = rules.filter((r) => r.id !== id);
  const isDefault = next.enabled && !next.from && !next.to && BUILTIN_RULES.some((b) => b.id === id);
  return isDefault ? rest : [...rest, next];
}

export type Errors = Record<string, string>;

/** Validação local antes de `save_config`; as chaves identificam o campo que mostra a mensagem. */
export function validateConfig(c: Config): Errors {
  const e: Errors = {};
  const t = c.transition;
  if (!Number.isInteger(t.dwell_ms) || t.dwell_ms < 0 || t.dwell_ms > 1000) e.dwell_ms = "Use um valor entre 0 e 1000 ms.";
  if (!(t.edge_resistance_pt >= 0)) e.edge_resistance_pt = "Não pode ser negativo.";
  if (t.corner_dead_zone_mm < 0) e.corner_dead_zone_mm = "Não pode ser negativo.";
  if (c.keymap.cmd_tap_timeout_ms < 1) e.cmd_tap_timeout_ms = "Deve ser maior que 0.";
  if (c.clipboard.max_bytes < 1) e.max_bytes = "Deve ser maior que 0.";
  if (c.clipboard.formats.length === 0) e.formats = "Escolha ao menos um formato.";
  if (c.audio.jitter_target_ms < 1) e.jitter_target_ms = "Deve ser maior que 0.";
  c.keymap.app_exceptions.forEach((x, i) => {
    if (!x.exe.trim()) e[`exception_${i}`] = "Informe o executável (ex.: Code.exe).";
  });
  for (const [k, others] of hotkeyConflicts(c.hotkeys)) e[`hotkey_${k}`] = `Conflita com: ${others.join(", ")}.`;
  return e;
}

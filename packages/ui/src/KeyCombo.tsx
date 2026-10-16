import type { KeyCombo as KeyComboValue, Modifier } from "@relay/protocol-types";
import styles from "./KeyCombo.module.css";

const MODIFIER_GLYPH: Record<Modifier, string> = { Ctrl: "⌃", Option: "⌥", Shift: "⇧", Cmd: "⌘" };
const MODIFIER_ORDER: Modifier[] = ["Ctrl", "Option", "Shift", "Cmd"];
const KEY_GLYPH: Record<string, string> = {
  ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→",
  Enter: "↩", Escape: "⎋", Backspace: "⌫", Tab: "⇥", Space: "Space",
};

/** Converte um `KeyboardEvent.code` ("KeyL", "Digit1", "F5") em rótulo curto. */
export function keyLabel(code: string): string {
  return KEY_GLYPH[code] ?? code.replace(/^(Key|Digit|Numpad)/, "");
}

export interface KeyComboProps {
  combo: KeyComboValue;
}

export function KeyCombo({ combo }: KeyComboProps) {
  const mods = MODIFIER_ORDER.filter((m) => combo.modifiers.includes(m));
  const parts = [...mods.map((m) => MODIFIER_GLYPH[m]), keyLabel(combo.key)];
  return (
    <span className={styles.combo} aria-label={[...mods, keyLabel(combo.key)].join(" ")}>
      {parts.map((p, i) => (
        <kbd key={i} className={styles.key} aria-hidden="true">
          {p}
        </kbd>
      ))}
    </span>
  );
}

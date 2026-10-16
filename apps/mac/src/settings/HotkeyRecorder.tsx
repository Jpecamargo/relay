// dono: U3. Gravador de combinação: clique, pressione as teclas (Esc cancela).
import { KeyCombo } from "@relay/ui";
import type { KeyCombo as KeyComboValue, Modifier } from "@relay/protocol-types";
import { useState, type KeyboardEvent } from "react";
import styles from "./settings.module.css";

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta)(Left|Right)$/;

export function comboFromEvent(e: Pick<KeyboardEvent, "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">): KeyComboValue | null {
  if (MODIFIER_CODES.test(e.code)) return null;
  const modifiers: Modifier[] = [];
  if (e.ctrlKey) modifiers.push("Ctrl");
  if (e.altKey) modifiers.push("Option");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey) modifiers.push("Cmd");
  return { modifiers, key: e.code };
}

export interface HotkeyRecorderProps {
  label: string;
  value: KeyComboValue | null;
  onChange: (combo: KeyComboValue | null) => void;
  error?: string;
  /** Exige ao menos um modificador (padrão dos atalhos globais). */
  requireModifier?: boolean;
}

export function HotkeyRecorder({ label, value, onChange, error, requireModifier = true }: HotkeyRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [note, setNote] = useState("");

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording) return;
    e.preventDefault();
    if (e.code === "Escape") {
      setRecording(false);
      setNote("");
      return;
    }
    const combo = comboFromEvent(e);
    if (!combo) return;
    if (requireModifier && combo.modifiers.length === 0) {
      setNote("Inclua ao menos um modificador.");
      return;
    }
    setRecording(false);
    setNote("");
    onChange(combo);
  };

  return (
    <div className={styles.recorder}>
      <button
        type="button"
        className={`${styles.recordBtn} ${recording ? styles.recording : ""}`}
        aria-label={`Gravar atalho: ${label}`}
        onClick={() => setRecording(true)}
        onKeyDown={onKeyDown}
        onBlur={() => setRecording(false)}
      >
        {recording ? "Pressione a combinação…" : value ? <KeyCombo combo={value} /> : <span className={styles.muted}>Sem atalho</span>}
      </button>
      {value ? (
        <button type="button" className={styles.linkBtn} aria-label={`Limpar atalho: ${label}`} onClick={() => onChange(null)}>
          Limpar
        </button>
      ) : null}
      {note || error ? (
        <span role="alert" className={styles.error}>
          {note || error}
        </span>
      ) : null}
    </div>
  );
}

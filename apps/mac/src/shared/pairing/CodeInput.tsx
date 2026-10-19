// Código de pareamento estilo MFA: 6 caixas, só dígitos, foco e envio automáticos.
import { useEffect, useRef, type ClipboardEvent, type KeyboardEvent } from "react";
import styles from "./pairing.module.css";

export const CODE_LENGTH = 6;
const BOXES = Array.from({ length: CODE_LENGTH }, (_, i) => i);

export interface CodeInputProps {
  value: string;
  onChange: (value: string) => void;
  /** Chamado quando o usuário completa o sexto dígito (digitando ou colando). */
  onComplete?: (code: string) => void;
  disabled?: boolean;
  invalid?: boolean;
  /** Foca a primeira caixa ao montar e sempre que o campo volta a ficar vazio. */
  autoFocus?: boolean;
  size?: "md" | "lg";
}

export function CodeInput({ value, onChange, onComplete, disabled, invalid, autoFocus, size = "md" }: CodeInputProps) {
  const refs = useRef<Array<HTMLInputElement | null>>([]);
  // O foco muda antes do React re-renderizar: o `onFocus` precisa do valor já digitado, não do da última renderização.
  const latest = useRef(value);
  latest.current = value;
  const focusBox = (i: number) => refs.current[Math.min(i, CODE_LENGTH - 1)]?.focus();

  useEffect(() => {
    if (autoFocus && !disabled && value === "") focusBox(0);
  }, [autoFocus, disabled, value]);

  const set = (next: string) => {
    latest.current = next;
    onChange(next);
  };

  const commit = (next: string) => {
    set(next);
    focusBox(next.length);
    if (next.length === CODE_LENGTH) onComplete?.(next);
  };

  const type = (i: number, raw: string) => {
    const digits = raw.replace(/\D/g, "");
    if (!digits) return;
    // Um dígito sobrescreve a caixa; vários (preenchimento automático do sistema) entram a partir dela.
    commit(digits.length === 1 ? value.slice(0, i) + digits + value.slice(i + 1) : (value.slice(0, i) + digits).slice(0, CODE_LENGTH));
  };

  const paste = (e: ClipboardEvent) => {
    e.preventDefault();
    const digits = e.clipboardData.getData("text").replace(/\D/g, "").slice(0, CODE_LENGTH);
    if (digits) commit(digits);
  };

  const keyDown = (i: number, e: KeyboardEvent) => {
    if (e.key === "Backspace") {
      e.preventDefault();
      if (i < value.length) {
        set(value.slice(0, i) + value.slice(i + 1));
      } else if (i > 0) {
        set(value.slice(0, i - 1));
        focusBox(i - 1);
      }
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      focusBox(Math.max(i - 1, 0));
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      focusBox(Math.min(i + 1, latest.current.length));
    }
  };

  return (
    <div role="group" aria-label="Código de 6 dígitos" className={`${styles.code} ${styles[size]}`}>
      {BOXES.map((i) => (
        <input
          key={i}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className={`${styles.box} ${invalid ? styles.invalid : ""}`}
          aria-label={`Dígito ${i + 1}`}
          aria-invalid={invalid || undefined}
          inputMode="numeric"
          autoComplete={i === 0 ? "one-time-code" : "off"}
          value={value[i] ?? ""}
          disabled={disabled}
          onChange={(e) => type(i, e.target.value)}
          onKeyDown={(e) => keyDown(i, e)}
          onPaste={paste}
          onFocus={(e) => {
            // Só se digita na primeira caixa vazia: clicar adiante volta para ela.
            if (i > latest.current.length) focusBox(latest.current.length);
            else e.target.select();
          }}
        />
      ))}
    </div>
  );
}

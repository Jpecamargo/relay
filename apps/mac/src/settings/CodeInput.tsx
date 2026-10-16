// dono: U3. Campo do código de pareamento: só dígitos, exatamente 6.
import { useId } from "react";
import styles from "./settings.module.css";

export const CODE_LENGTH = 6;

export interface CodeInputProps {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}

export function CodeInput({ value, onChange, disabled }: CodeInputProps) {
  const id = useId();
  const hint = value.length > 0 && value.length < CODE_LENGTH ? `Faltam ${CODE_LENGTH - value.length} dígitos.` : "";
  return (
    <div className={styles.field}>
      <label htmlFor={id} className={styles.fieldLabel}>
        Código de 6 dígitos
      </label>
      <input
        id={id}
        className={`${styles.input} ${styles.code}`}
        inputMode="numeric"
        autoComplete="one-time-code"
        maxLength={CODE_LENGTH}
        placeholder="000000"
        value={value}
        disabled={disabled}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onChange(e.target.value.replace(/\D/g, "").slice(0, CODE_LENGTH))}
      />
      <span id={`${id}-hint`} className={styles.hint}>
        {hint}
      </span>
    </div>
  );
}

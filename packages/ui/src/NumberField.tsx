import { useEffect, useId, useState } from "react";
import styles from "./NumberField.module.css";

export interface NumberFieldProps {
  value: number;
  onChange: (value: number) => void;
  label: string;
  min?: number;
  max?: number;
  step?: number;
  /** Unidade mostrada depois do campo ("ms", "px"). */
  unit?: string;
  disabled?: boolean;
}

export function NumberField({ value, onChange, label, min, max, step = 1, unit, disabled }: NumberFieldProps) {
  const id = useId();
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);

  const commit = () => {
    const parsed = Number(text);
    if (text.trim() === "" || Number.isNaN(parsed)) {
      setText(String(value));
      return;
    }
    const clamped = Math.min(max ?? Infinity, Math.max(min ?? -Infinity, parsed));
    setText(String(clamped));
    if (clamped !== value) onChange(clamped);
  };

  return (
    <div className={styles.row}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <span className={styles.field}>
        <input
          id={id}
          type="number"
          inputMode="decimal"
          className={styles.input}
          value={text}
          min={min}
          max={max}
          step={step}
          disabled={disabled}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
        {unit ? <span className={styles.unit}>{unit}</span> : null}
      </span>
    </div>
  );
}

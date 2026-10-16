import { useEffect, useId, useRef, useState } from "react";
import styles from "./Slider.module.css";

export interface SliderProps {
  value: number;
  /** Chamado depois de `debounceMs` sem novo movimento, com o último valor. */
  onChange: (value: number) => void;
  label: string;
  min?: number;
  max?: number;
  step?: number;
  /** Espera antes de chamar `onChange` (padrão 150 ms). 0 chama a cada movimento. */
  debounceMs?: number;
  /** Formata o número mostrado ao lado (padrão: o próprio valor). */
  format?: (value: number) => string;
  disabled?: boolean;
}

export function Slider({
  value,
  onChange,
  label,
  min = 0,
  max = 100,
  step = 1,
  debounceMs = 150,
  format,
  disabled,
}: SliderProps) {
  const id = useId();
  const [local, setLocal] = useState(value);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => setLocal(value), [value]);
  useEffect(() => () => clearTimeout(timer.current), []);

  const handle = (next: number) => {
    setLocal(next);
    clearTimeout(timer.current);
    if (debounceMs <= 0) onChange(next);
    else timer.current = setTimeout(() => onChange(next), debounceMs);
  };

  return (
    <div className={styles.row}>
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <input
        id={id}
        type="range"
        className={styles.range}
        min={min}
        max={max}
        step={step}
        value={local}
        disabled={disabled}
        onChange={(e) => handle(Number(e.target.value))}
      />
      <output htmlFor={id} className={styles.value}>
        {format ? format(local) : local}
      </output>
    </div>
  );
}

import styles from "./ProgressBar.module.css";

export interface ProgressBarProps {
  /** Valor atual, de 0 a `max`. */
  value: number;
  max?: number;
  label: string;
}

export function ProgressBar({ value, max = 1, label }: ProgressBarProps) {
  const ratio = max > 0 ? Math.min(1, Math.max(0, value / max)) : 0;
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={Math.min(max, Math.max(0, value))}
      className={styles.track}
    >
      <div className={styles.fill} style={{ width: `${ratio * 100}%` }} />
    </div>
  );
}
